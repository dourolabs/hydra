use crate::app::AppState;
use crate::app::chat_relay;
use crate::domain::actors::{Actor, ActorRef};
use crate::store::StoreError;
use axum::{
    Extension,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use hydra_common::SessionId;
use hydra_common::api::v1::conversations::{
    ConversationEvent, ServerMessage, WorkerCatchUp, WorkerConnect, WorkerMessage,
};
use tracing::{error, info, warn};

use super::{ApiError, SessionIdPath};

/// GET /v1/sessions/:session_id/relay — WebSocket upgrade endpoint for
/// the interactive chat relay. Workers connect here to exchange messages
/// with the server.
pub async fn session_relay(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Verify the session exists and has a conversation_id.
    let session = state
        .get_latest_session(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => ApiError::internal(format!("failed to load session: {other}")),
        })?;

    let conversation_id = session.conversation_id().cloned().ok_or_else(|| {
        ApiError::bad_request(format!(
            "session '{session_id}' is not an interactive session (no conversation_id)"
        ))
    })?;

    // Verify the conversation exists.
    state
        .store()
        .get_conversation(&conversation_id, false)
        .await
        .map_err(|err| match err {
            StoreError::ConversationNotFound(_) => {
                ApiError::not_found(format!("conversation '{conversation_id}' not found"))
            }
            other => ApiError::internal(format!("failed to load conversation: {other}")),
        })?;

    info!(
        session_id = %session_id,
        conversation_id = %conversation_id,
        actor = %actor.name(),
        "WebSocket relay upgrade requested"
    );

    Ok(ws.on_upgrade(move |socket| {
        handle_relay_socket(socket, state, session_id, conversation_id, actor)
    }))
}

async fn handle_relay_socket(
    socket: WebSocket,
    state: AppState,
    session_id: SessionId,
    conversation_id: hydra_common::ConversationId,
    actor: Actor,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Step 1: Wait for WorkerConnect handshake message.
    let worker_connect = match ws_receiver.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<WorkerConnect>(&text) {
            Ok(msg) => msg,
            Err(err) => {
                error!(%session_id, error = %err, "invalid WorkerConnect message");
                let _ = ws_sender.send(Message::Close(None)).await;
                return;
            }
        },
        Some(Ok(Message::Close(_))) | None => {
            info!(%session_id, "WebSocket closed before handshake");
            return;
        }
        Some(Ok(other)) => {
            error!(%session_id, msg_type = ?other, "expected text WorkerConnect message");
            let _ = ws_sender.send(Message::Close(None)).await;
            return;
        }
        Some(Err(err)) => {
            error!(%session_id, error = %err, "WebSocket error during handshake");
            return;
        }
    };

    info!(
        %session_id,
        connect = ?worker_connect,
        "worker connected, performing catch-up"
    );

    // Step 2: Build WorkerCatchUp response.
    let catch_up = match build_catch_up(&state, &conversation_id, &worker_connect).await {
        Ok(catch_up) => catch_up,
        Err(err) => {
            error!(%session_id, error = %err, "failed to build catch-up");
            let _ = ws_sender.send(Message::Close(None)).await;
            return;
        }
    };

    // Send catch-up to worker.
    let catch_up_msg = ServerMessage::CatchUp(catch_up);
    let catch_up_json = match serde_json::to_string(&catch_up_msg) {
        Ok(json) => json,
        Err(err) => {
            error!(%session_id, error = %err, "failed to serialize catch-up");
            return;
        }
    };
    if ws_sender.send(Message::Text(catch_up_json)).await.is_err() {
        warn!(%session_id, "failed to send catch-up, WebSocket closed");
        return;
    }

    // Step 3: Register relay in ChatRelayMap.
    let mut user_msg_rx = chat_relay::register_relay(
        &state.chat_relay_map,
        conversation_id.clone(),
        session_id.clone(),
    );

    info!(%session_id, "relay registered, starting relay loop");

    let actor_ref = ActorRef::from(&actor);

    // Step 4: Relay loop — bidirectional message forwarding.
    loop {
        tokio::select! {
            // Worker -> Server: messages from WebSocket
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WorkerMessage>(&text) {
                            Ok(WorkerMessage::Event { event }) => {
                                if let Err(err) = handle_worker_event(
                                    &state,
                                    &conversation_id,
                                    &actor_ref,
                                    event,
                                ).await {
                                    error!(%session_id, error = %err, "failed to handle worker event");
                                }
                            }
                            Ok(WorkerMessage::SessionStateUpload { data }) => {
                                let bytes = data.len();
                                info!(
                                    %session_id,
                                    %conversation_id,
                                    bytes,
                                    "received SessionStateUpload — storing"
                                );
                                match state
                                    .store
                                    .store_conversation_session_state(&conversation_id, data)
                                    .await
                                {
                                    Ok(()) => info!(
                                        %session_id,
                                        %conversation_id,
                                        bytes,
                                        "session_state stored"
                                    ),
                                    Err(err) => error!(
                                        %session_id,
                                        %conversation_id,
                                        bytes,
                                        error = %err,
                                        "failed to store session state"
                                    ),
                                }
                            }
                            Err(err) => {
                                warn!(%session_id, error = %err, "invalid worker message, ignoring");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(%session_id, "WebSocket closed by worker");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(_)) => {
                        // Ignore binary and pong messages
                    }
                    Some(Err(err)) => {
                        error!(%session_id, error = %err, "WebSocket error in relay loop");
                        break;
                    }
                }
            }

            // Server -> Worker: user messages queued via the relay
            user_msg = user_msg_rx.recv() => {
                match user_msg {
                    Some(event) => {
                        let server_msg = ServerMessage::Event { event };
                        match serde_json::to_string(&server_msg) {
                            Ok(json) => {
                                if ws_sender.send(Message::Text(json)).await.is_err() {
                                    warn!(%session_id, "failed to forward user message, WebSocket closed");
                                    break;
                                }
                            }
                            Err(err) => {
                                error!(%session_id, error = %err, "failed to serialize server message");
                            }
                        }
                    }
                    None => {
                        info!(%session_id, "user message channel closed");
                        break;
                    }
                }
            }
        }
    }

    // Step 5: Cleanup on disconnect. The conversation status (Active → Idle)
    // is owned by `SpawnConversationSessionsAutomation`, which flips it when
    // the companion session reaches a terminal status (Complete / Failed).
    // The relay only unregisters its in-memory entry here.
    chat_relay::unregister_relay(&state.chat_relay_map, &conversation_id);
    info!(%session_id, %conversation_id, "relay unregistered");
}

/// Build the WorkerCatchUp payload based on the worker's connect message.
///
/// For `Fresh` connections, we always return the full event log: a Fresh
/// handshake means a brand-new worker process (typically in a new container
/// for resume) that needs the entire conversation history to rebuild context.
/// The `resume_from_event_index` field is ignored — see the resume design in
/// `hydra/src/worker/interactive.rs` for how the worker reconstructs context
/// from the replayed events. We also include any persisted `session_state` so
/// the worker can attempt the primary transcript-based resume; if missing or
/// malformed, the worker falls back to the primer-from-events path.
///
/// For `Reconnecting` connections, we keep the skip behavior. That path is a
/// mid-session WebSocket reconnect where the same worker process is still
/// alive and only needs the deltas it missed. `session_state` is not relevant
/// there and is omitted.
async fn build_catch_up(
    state: &AppState,
    conversation_id: &hydra_common::ConversationId,
    worker_connect: &WorkerConnect,
) -> Result<WorkerCatchUp, StoreError> {
    let all_events = state
        .store()
        .get_conversation_events(conversation_id)
        .await?;

    let (skip_count, include_session_state) = match worker_connect {
        WorkerConnect::Fresh { .. } => (0, true),
        WorkerConnect::Reconnecting {
            last_received_event_index,
        } => (last_received_event_index + 1, false),
    };

    let events: Vec<ConversationEvent> = all_events
        .into_iter()
        .skip(skip_count)
        .map(|v| v.item.into())
        .collect();

    let session_state = if include_session_state {
        state
            .store()
            .get_conversation_session_state(conversation_id)
            .await?
    } else {
        None
    };

    let session_state_bytes = session_state.as_ref().map(|b| b.len());
    info!(
        %conversation_id,
        events = events.len(),
        include_session_state,
        session_state_bytes = ?session_state_bytes,
        "build_catch_up"
    );

    Ok(WorkerCatchUp {
        events,
        session_state,
    })
}

/// Handle a conversation event sent by the worker.
///
/// The `Suspending` event is recorded in the conversation log (worker's
/// record of why it suspended) but does NOT mutate the conversation's status
/// here. The worker exiting after a Suspending event lets the job engine drive
/// the session to `Complete` / `Failed`; `SpawnConversationSessionsAutomation`
/// then flips the conversation `Active → Idle` from that terminal transition.
async fn handle_worker_event(
    state: &AppState,
    conversation_id: &hydra_common::ConversationId,
    actor_ref: &ActorRef,
    event: ConversationEvent,
) -> Result<(), StoreError> {
    let domain_event: crate::domain::conversations::ConversationEvent = event.into();
    state
        .store
        .append_conversation_event_with_actor(conversation_id, domain_event, actor_ref.clone())
        .await?;
    Ok(())
}
