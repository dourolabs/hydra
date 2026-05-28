use crate::app::AppState;
use crate::app::chat_relay;
use crate::domain::actors::{Actor, ActorRef};
use crate::store::StoreError;
use axum::{
    Extension, Json,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
};
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use hydra_common::SessionId;
use hydra_common::api::v1;
use hydra_common::api::v1::conversations::{ServerMessage, WorkerConnect, WorkerMessage};
use hydra_common::api::v1::sessions::SessionEvent;
use tracing::{debug, error, info, warn};

use super::{ApiError, SessionIdPath};

/// GET /v1/sessions/:session_id/events
///
/// Dual-mode handler:
/// * If the request carries a WebSocket `Upgrade` header → upgrade to the
///   per-session worker relay (renamed from the legacy `/relay` route per
///   the PR-3 cutover).
/// * Otherwise → return the persisted `SessionEvent` log as JSON.
pub async fn session_events_or_ws(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    ws: Option<WebSocketUpgrade>,
) -> Result<Response, ApiError> {
    if let Some(ws) = ws {
        session_relay_inner(state, actor, session_id, ws).await
    } else {
        get_session_events_json(state, session_id).await
    }
}

async fn get_session_events_json(
    state: AppState,
    session_id: SessionId,
) -> Result<Response, ApiError> {
    info!(session_id = %session_id, "get_session_events invoked");
    let events = state
        .store()
        .get_session_events(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => ApiError::internal(format!(
                "Failed to load session events '{session_id}': {other}"
            )),
        })?;
    let api_events: Vec<v1::sessions::SessionEvent> =
        events.into_iter().map(|v| v.item.into()).collect();
    Ok(Json(api_events).into_response())
}

async fn session_relay_inner(
    state: AppState,
    actor: Actor,
    session_id: SessionId,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Verify the session exists. Both interactive and headless modes share
    // the same WS route now (the legacy interactive-only guard is gone per
    // PR-3 design §5).
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
        ApiError::internal(format!("session '{session_id}' has no conversation_id"))
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
        "events WebSocket upgrade requested"
    );

    let session_value = session.clone();
    let greet_user = session.mode.greet_user();
    let resumed_from = session.resumed_from.clone();
    let system_prompt = session.agent_config.system_prompt.clone();

    Ok(ws.on_upgrade(move |socket| {
        handle_relay_socket(
            socket,
            state,
            session_id,
            conversation_id,
            actor,
            greet_user,
            resumed_from,
            system_prompt,
            session_value,
        )
    }))
}

/// Per-WS-connection state used by Phase 2 ([`ServerMessage::FirstMessage`]
/// dispatch). When the worker sends `Ready` and there is no `UserMessage`
/// yet, the server stashes the resolved `agent_prompt` here and the existing
/// UserMessage event-arrival hook clears the flag once one lands.
#[derive(Debug)]
struct PendingFirstMessage {
    agent_prompt: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn handle_relay_socket(
    socket: WebSocket,
    state: AppState,
    session_id: SessionId,
    conversation_id: hydra_common::ConversationId,
    actor: Actor,
    greet_user: bool,
    resumed_from: Option<SessionId>,
    system_prompt: Option<String>,
    _session: crate::domain::sessions::Session,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // ---- Phase 1: handshake ----
    let worker_connect = match recv_worker_connect(&mut ws_receiver, &session_id).await {
        Some(c) => c,
        None => {
            let _ = ws_sender.send(Message::Close(None)).await;
            return;
        }
    };

    info!(
        %session_id,
        connect = ?worker_connect,
        "worker connected"
    );

    let mut pending_first_message: Option<PendingFirstMessage> = None;

    match worker_connect {
        WorkerConnect::Fresh => {
            // Look up the prior session's state blob if this session is a
            // resume continuation.
            let resume_blob = if let Some(prior_id) = resumed_from.as_ref() {
                match state.store().get_session_state(prior_id).await {
                    Ok(blob) => blob,
                    Err(err) => {
                        warn!(
                            %session_id,
                            prior_session_id = %prior_id,
                            error = %err,
                            "failed to load prior session_state blob; worker will fall back to transcript"
                        );
                        None
                    }
                }
            } else {
                None
            };

            let resume_msg = ServerMessage::ResumeContext {
                resume_blob,
                prior_session_id: resumed_from.clone(),
            };
            if !send_server_msg(&mut ws_sender, &session_id, &resume_msg).await {
                return;
            }
        }
        WorkerConnect::Reconnecting {
            last_received_session_event_index,
        } => {
            // Mid-session reconnect: ship every session event past the
            // worker's index. Phase 2 is skipped — the model is already
            // running.
            let events = match collect_session_events_after(
                &state,
                &session_id,
                last_received_session_event_index,
            )
            .await
            {
                Ok(events) => events,
                Err(err) => {
                    error!(%session_id, error = %err, "failed to collect catch-up events");
                    let _ = ws_sender.send(Message::Close(None)).await;
                    return;
                }
            };
            let catch_up_msg = ServerMessage::CatchUp { events };
            if !send_server_msg(&mut ws_sender, &session_id, &catch_up_msg).await {
                return;
            }
        }
    }

    // ---- Register the relay so user-message writes flow to the worker ----
    let mut user_msg_rx = chat_relay::register_relay(
        &state.chat_relay_map,
        conversation_id.clone(),
        session_id.clone(),
    );

    info!(%session_id, "relay registered, starting relay loop");

    let actor_ref = ActorRef::from(&actor);

    // ---- Phase 2 / 3: main loop ----
    loop {
        tokio::select! {
            // Worker -> Server
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WorkerMessage>(&text) {
                            Ok(WorkerMessage::Connect(_)) => {
                                warn!(%session_id, "received unexpected duplicate Connect message; ignoring");
                            }
                            Ok(WorkerMessage::RequestTranscript { prior_session_id }) => {
                                let events = match state.store().get_session_events(&prior_session_id).await {
                                    Ok(versioned) => versioned.into_iter().map(|v| v.item.into()).collect(),
                                    Err(err) => {
                                        warn!(
                                            %session_id,
                                            %prior_session_id,
                                            error = %err,
                                            "failed to load transcript; sending empty"
                                        );
                                        Vec::new()
                                    }
                                };
                                let msg = ServerMessage::Transcript { events };
                                if !send_server_msg(&mut ws_sender, &session_id, &msg).await {
                                    break;
                                }
                            }
                            Ok(WorkerMessage::Ready) => {
                                // Resolve agent_prompt per design §1.5.
                                let agent_prompt = if resumed_from.is_some() {
                                    None
                                } else {
                                    system_prompt.clone()
                                };

                                // Look up first SessionEvent::UserMessage on this session's
                                // conversation (across the whole conversation's session chain
                                // for resumed cases — though resumed sessions are gated above).
                                let user_message = match find_first_user_message(
                                    &state, &conversation_id,
                                ).await {
                                    Ok(opt) => opt,
                                    Err(err) => {
                                        error!(%session_id, error = %err, "failed to look up first UserMessage for FirstMessage");
                                        None
                                    }
                                };

                                // Snapshot the current count of session_events for THIS
                                // session — what the worker should consider "already seen"
                                // before Phase-3 traffic begins. See design §1.5 / §1.6.
                                let session_event_baseline =
                                    current_session_event_count(&state, &session_id).await;

                                if let Some(content) = user_message {
                                    let msg = ServerMessage::FirstMessage {
                                        agent_prompt,
                                        user_message: Some(content),
                                        session_event_baseline,
                                    };
                                    if !send_server_msg(&mut ws_sender, &session_id, &msg).await {
                                        break;
                                    }
                                } else if greet_user {
                                    let msg = ServerMessage::FirstMessage {
                                        agent_prompt,
                                        user_message: None,
                                        session_event_baseline,
                                    };
                                    if !send_server_msg(&mut ws_sender, &session_id, &msg).await {
                                        break;
                                    }
                                } else {
                                    // Stash the agent_prompt; the UserMessage event-arrival
                                    // hook (the `user_msg_rx` branch below) will drain it.
                                    // Re-snapshot the baseline at that moment so it covers
                                    // the just-arrived UserMessage's index.
                                    pending_first_message = Some(PendingFirstMessage { agent_prompt });
                                    debug!(%session_id, "FirstMessage deferred — awaiting first UserMessage");
                                }
                            }
                            Ok(WorkerMessage::Event { event }) => {
                                if let Err(err) = handle_worker_event(
                                    &state,
                                    &conversation_id,
                                    &session_id,
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
                                let _ = chat_relay::store_session_state(
                                    &state,
                                    &session_id,
                                    data,
                                    actor_ref.clone(),
                                )
                                .await;
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

            // Server -> Worker: user messages queued via the relay.
            //
            // When `pending_first_message.is_some()`, the first arriving
            // `SessionEvent::UserMessage` triggers a `FirstMessage` emit and
            // suppresses the normal Phase-3 `Event` push for that same
            // message (its content goes inside `user_message`). All
            // subsequent UserMessages flow through the normal path.
            user_msg = user_msg_rx.recv() => {
                match user_msg {
                    Some(event) => {
                        if let Some(pending) = pending_first_message.take() {
                            if let SessionEvent::UserMessage { content, .. } = &event {
                                // The UserMessage was already appended to session_events
                                // by the POST /messages writer before being pushed onto
                                // this channel, so the baseline includes it.
                                let session_event_baseline =
                                    current_session_event_count(&state, &session_id).await;
                                let msg = ServerMessage::FirstMessage {
                                    agent_prompt: pending.agent_prompt,
                                    user_message: Some(content.clone()),
                                    session_event_baseline,
                                };
                                if !send_server_msg(&mut ws_sender, &session_id, &msg).await {
                                    break;
                                }
                                // Suppress the Phase-3 push of THIS UserMessage —
                                // its content is already inside FirstMessage.
                                continue;
                            } else {
                                // Non-UserMessage arrived first; keep the flag stashed.
                                pending_first_message = Some(pending);
                            }
                        }
                        let server_msg = ServerMessage::Event { event };
                        if !send_server_msg(&mut ws_sender, &session_id, &server_msg).await {
                            break;
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

    // Cleanup on disconnect.
    chat_relay::unregister_relay(&state.chat_relay_map, &conversation_id);
    info!(%session_id, %conversation_id, "relay unregistered");
}

async fn recv_worker_connect(
    ws_receiver: &mut futures::stream::SplitStream<WebSocket>,
    session_id: &SessionId,
) -> Option<WorkerConnect> {
    match ws_receiver.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<WorkerMessage>(&text) {
            Ok(WorkerMessage::Connect(c)) => Some(c),
            Ok(other) => {
                error!(%session_id, msg = ?other, "expected WorkerMessage::Connect first");
                None
            }
            Err(err) => {
                error!(%session_id, error = %err, "invalid handshake message");
                None
            }
        },
        Some(Ok(Message::Close(_))) | None => {
            info!(%session_id, "WebSocket closed before handshake");
            None
        }
        Some(Ok(other)) => {
            error!(%session_id, msg_type = ?other, "expected text WorkerMessage::Connect");
            None
        }
        Some(Err(err)) => {
            error!(%session_id, error = %err, "WebSocket error during handshake");
            None
        }
    }
}

async fn send_server_msg(
    ws_sender: &mut SplitSink<WebSocket, Message>,
    session_id: &SessionId,
    msg: &ServerMessage,
) -> bool {
    let json = match serde_json::to_string(msg) {
        Ok(j) => j,
        Err(err) => {
            error!(%session_id, error = %err, "failed to serialize ServerMessage");
            return false;
        }
    };
    if ws_sender.send(Message::Text(json)).await.is_err() {
        warn!(%session_id, "failed to send ServerMessage, WebSocket closed");
        return false;
    }
    true
}

/// Read `session_events.len()` for a single session id. Used to populate
/// `ServerMessage::FirstMessage.session_event_baseline` so the worker's
/// running event count matches the server's `session_events.index` convention
/// before Phase 3 begins. Returns `0` on a store error (the worst that can
/// happen is a single duplicated `UserMessage` on reconnect — same shape as
/// the pre-fix bug — and we want the protocol to be resilient to a transient
/// store hiccup at this point).
async fn current_session_event_count(state: &AppState, session_id: &SessionId) -> usize {
    match state.store().get_session_events(session_id).await {
        Ok(events) => events.len(),
        Err(err) => {
            warn!(
                %session_id,
                error = %err,
                "failed to read session_events.len() for FirstMessage.session_event_baseline; \
                 falling back to 0"
            );
            0
        }
    }
}

/// Collect session events on `session_id` whose index is strictly greater
/// than `last_received_session_event_index`. Per-session indexes are dense
/// and monotonic, starting at 0.
async fn collect_session_events_after(
    state: &AppState,
    session_id: &SessionId,
    last_received_session_event_index: usize,
) -> Result<Vec<SessionEvent>, StoreError> {
    let events = state.store().get_session_events(session_id).await?;
    Ok(events
        .into_iter()
        .skip(last_received_session_event_index.saturating_add(1))
        .map(|v| v.item.into())
        .collect())
}

/// Find the first `SessionEvent::UserMessage` content across every session
/// linked to the conversation, in session creation-time order. Returns
/// `None` if no UserMessage has been recorded yet.
///
/// Worst-case cost is O(N) serial `get_session_events` reads over the
/// conversation's session chain. The loop short-circuits the moment it finds
/// the first UserMessage, so the common case is a single read; long
/// resumption chains (suspend/resume across many sessions) bear the full
/// fan-out cost. A dedicated `store::get_first_user_message_for_conversation`
/// could collapse this to one read; left as a follow-up because the call site
/// is a one-per-WS-handshake cold path and the speedup is moot for the
/// typical 1–2 session chains we see today.
async fn find_first_user_message(
    state: &AppState,
    conversation_id: &hydra_common::ConversationId,
) -> Result<Option<String>, StoreError> {
    let mut query = hydra_common::api::v1::sessions::SearchSessionsQuery::default();
    query.conversation_id = Some(conversation_id.clone());
    let mut sessions = state.store().list_sessions(&query).await?;
    sessions.sort_by_key(|(_, v)| v.creation_time);
    for (sid, _) in &sessions {
        let events = state.store().get_session_events(sid).await?;
        for v in events {
            if let crate::domain::sessions::SessionEvent::UserMessage { content, .. } = &v.item {
                return Ok(Some(content.clone()));
            }
        }
    }
    Ok(None)
}

/// Handle a session event sent by the worker.
async fn handle_worker_event(
    state: &AppState,
    conversation_id: &hydra_common::ConversationId,
    session_id: &SessionId,
    actor_ref: &ActorRef,
    event: SessionEvent,
) -> Result<(), StoreError> {
    let domain_event: crate::domain::sessions::SessionEvent = match event.try_into() {
        Ok(e) => e,
        Err(_) => {
            warn!(%session_id, "worker emitted an unknown SessionEvent variant; ignoring");
            return Ok(());
        }
    };
    state
        .store
        .append_session_event_with_actor(session_id, domain_event.clone(), actor_ref.clone())
        .await?;

    // Mirror lifecycle events (Suspending / Closed) onto the conversation
    // events log so the conversation's lifecycle history stays observable
    // through the legacy `ConversationEvent` SSE / read paths.
    if let Some(conv_event) = session_event_to_lifecycle_conversation_event(&domain_event) {
        let _ = state
            .store
            .append_conversation_event_with_actor(conversation_id, conv_event, actor_ref.clone())
            .await;
    }
    Ok(())
}

/// Map a worker-emitted [`SessionEvent`] onto the corresponding lifecycle
/// [`crate::domain::conversations::ConversationEvent`], if any. Used by the
/// relay handler to mirror Suspending / Closed onto the conversation events
/// log. Returns `None` for chat-content variants.
fn session_event_to_lifecycle_conversation_event(
    event: &crate::domain::sessions::SessionEvent,
) -> Option<crate::domain::conversations::ConversationEvent> {
    use crate::domain::conversations::ConversationEvent as CE;
    use crate::domain::sessions::SessionEvent as SE;
    match event {
        SE::Suspending { reason, timestamp } => Some(CE::Suspending {
            reason: reason.clone(),
            timestamp: *timestamp,
        }),
        SE::Closed { timestamp } => Some(CE::Closed {
            timestamp: *timestamp,
        }),
        _ => None,
    }
}
