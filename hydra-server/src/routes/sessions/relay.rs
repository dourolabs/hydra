use crate::app::AppState;
use crate::app::chat_relay::TO_WORKER_CAPACITY;
use crate::domain::actors::{Actor, ActorRef};
use crate::domain::sessions::SessionMode;
use crate::store::{ReadOnlyStore, StoreError};
use axum::{
    Extension,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt, stream::SplitSink};
use hydra_common::ConversationId;
use hydra_common::SessionId;
use hydra_common::api::v1::conversations::{CatchUpEvent, ServerMessage, WorkerMessage};
use hydra_common::api::v1::sessions::SessionEvent;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::{ApiError, SessionIdPath};

/// GET /v1/sessions/:session_id/events — dual-purpose endpoint.
///
/// * With a WebSocket `Upgrade` header, this becomes the WS-only worker
///   lifecycle (Phase 1 negotiate → Phase 2 first message → Phase 3
///   bidirectional pump). Both interactive and headless sessions land on
///   this branch; mode-specific behaviour is confined to `handle_ready`.
/// * Without an `Upgrade` header, this returns the persisted
///   `SessionEvent` log as JSON — the read path used by
///   `useChatTranscript` and CLI tooling.
pub async fn session_events_or_relay(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    ws: Option<WebSocketUpgrade>,
) -> Result<Response, ApiError> {
    match ws {
        Some(ws) => {
            session_relay(
                State(state),
                Extension(actor),
                SessionIdPath(session_id),
                ws,
            )
            .await
        }
        None => session_events_json(State(state), SessionIdPath(session_id))
            .await
            .map(axum::response::IntoResponse::into_response),
    }
}

/// JSON read of a session's persisted `SessionEvent` log.
async fn session_events_json(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<axum::Json<Vec<SessionEvent>>, ApiError> {
    use crate::store::ReadOnlyStore as _;
    let events = state
        .store
        .get_session_events(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => ApiError::internal(format!("failed to load session events: {other}")),
        })?;
    let api_events: Vec<SessionEvent> = events.into_iter().map(|v| v.item.into()).collect();
    Ok(axum::Json(api_events))
}

/// WebSocket upgrade endpoint for the WS-only worker lifecycle.
async fn session_relay(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let session = state
        .get_latest_session(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => ApiError::internal(format!("failed to load session: {other}")),
        })?;

    info!(
        session_id = %session_id,
        mode = ?session.mode,
        actor = %actor.name(),
        "WebSocket events upgrade requested"
    );

    Ok(ws.on_upgrade(move |socket| handle_events_socket(socket, state, session_id, actor)))
}

async fn handle_events_socket(
    socket: WebSocket,
    state: AppState,
    session_id: SessionId,
    actor: Actor,
) {
    let session = match state.get_latest_session(&session_id).await {
        Ok(s) => s,
        Err(err) => {
            error!(%session_id, error = %err, "failed to load session after upgrade");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let actor_ref = ActorRef::from(&actor);

    // Phase 1: read the first inbound message — it must be Fresh or
    // Reconnecting; anything else is a protocol error.
    let first = match ws_receiver.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<WorkerMessage>(&text) {
            Ok(msg) => msg,
            Err(err) => {
                error!(%session_id, error = %err, "invalid first WorkerMessage");
                let _ = ws_sender.send(Message::Close(None)).await;
                return;
            }
        },
        Some(Ok(Message::Close(_))) | None => {
            info!(%session_id, "WebSocket closed before first message");
            return;
        }
        Some(Ok(other)) => {
            error!(%session_id, msg_type = ?other, "expected text WorkerMessage");
            let _ = ws_sender.send(Message::Close(None)).await;
            return;
        }
        Some(Err(err)) => {
            error!(%session_id, error = %err, "WebSocket error during handshake");
            return;
        }
    };

    match first {
        WorkerMessage::Fresh => {
            handle_fresh_path(
                ws_sender,
                ws_receiver,
                state,
                session_id,
                session,
                actor_ref,
            )
            .await
        }
        WorkerMessage::Reconnecting {
            last_received_session_event_index,
        } => {
            handle_reconnecting_path(
                ws_sender,
                ws_receiver,
                state,
                session_id,
                session,
                last_received_session_event_index,
                actor_ref,
            )
            .await
        }
        other => {
            error!(
                %session_id,
                ?other,
                "first WorkerMessage must be Fresh or Reconnecting; bailing connection"
            );
            let _ = ws_sender.send(Message::Close(None)).await;
        }
    }
}

async fn handle_fresh_path(
    mut ws_sender: SplitSink<WebSocket, Message>,
    mut ws_receiver: futures::stream::SplitStream<WebSocket>,
    state: AppState,
    session_id: SessionId,
    session: crate::domain::sessions::Session,
    actor_ref: ActorRef,
) {
    // Reply ResumeContext from the session_state store.
    let resume_blob = match state.store.get_session_state(&session_id).await {
        Ok(blob) => blob,
        Err(err) => {
            warn!(%session_id, error = %err, "failed to load session_state for ResumeContext");
            None
        }
    };
    let prior_session_id = session.resumed_from.clone();
    let resume_ctx = ServerMessage::ResumeContext {
        resume_blob,
        prior_session_id,
    };
    if !send_json(&mut ws_sender, &resume_ctx).await {
        return;
    }

    // The Phase-1 RequestTranscript fallback may arrive before Ready —
    // we loop on inbound messages until we see Ready (or another
    // expected late-phase variant).
    let conversation_id = session.conversation_id().cloned();

    // Register the active connection so subsequent UserMessages on this
    // conversation reach us via the relay. Headless sessions skip this.
    let (to_worker, mut to_worker_rx) = mpsc::channel::<ServerMessage>(TO_WORKER_CAPACITY);
    let mut drained_pending: Vec<(SessionEvent, usize)> = Vec::new();
    if let Some(conv_id) = conversation_id.as_ref() {
        drained_pending = state
            .chat_relay_map
            .set_active(conv_id.clone(), session_id.clone(), to_worker, &state.store)
            .await;
    }

    // Forward any pre-Ready drained UserMessages as Event pushes;
    // they'll be re-folded into FirstMessage at the handle_ready step if
    // the mode calls for it. (In practice this happens later via
    // pending_first_message — drained messages here are the rare case
    // where a UserMessage arrived before set_active.)
    let mut drained_user_messages: Vec<String> = Vec::new();
    for (event, event_index) in drained_pending {
        if let SessionEvent::UserMessage { content, .. } = &event {
            drained_user_messages.push(content.clone());
        }
        let msg = ServerMessage::Event { event, event_index };
        if !send_json(&mut ws_sender, &msg).await {
            cleanup(&state, conversation_id.as_ref());
            return;
        }
    }

    // Phase-1/Phase-2 loop: handle RequestTranscript fallback and Ready.
    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WorkerMessage>(&text) {
                            Ok(WorkerMessage::RequestTranscript { prior_session_id }) => {
                                let events = match collect_transcript(&state, &prior_session_id).await {
                                    Ok(events) => events,
                                    Err(err) => {
                                        error!(%session_id, error = %err, "transcript load failed");
                                        let _ = ws_sender.send(Message::Close(None)).await;
                                        cleanup(&state, conversation_id.as_ref());
                                        return;
                                    }
                                };
                                let transcript = ServerMessage::Transcript { events };
                                if !send_json(&mut ws_sender, &transcript).await {
                                    cleanup(&state, conversation_id.as_ref());
                                    return;
                                }
                            }
                            Ok(WorkerMessage::Ready) => {
                                if !handle_ready(
                                    &mut ws_sender,
                                    &state,
                                    &session_id,
                                    &session,
                                    drained_user_messages.first().cloned(),
                                )
                                .await
                                {
                                    cleanup(&state, conversation_id.as_ref());
                                    return;
                                }
                                break;
                            }
                            // Phase-3 variants arriving before Ready are
                            // tolerated; the worker may begin emitting
                            // events as soon as Phase 1 completes.
                            Ok(WorkerMessage::Event { event }) => {
                                if let Err(err) = handle_worker_event(
                                    &state,
                                    conversation_id.as_ref(),
                                    &session_id,
                                    &actor_ref,
                                    event,
                                ).await {
                                    error!(%session_id, error = %err, "failed to handle worker event in Phase 1/2");
                                }
                            }
                            Ok(WorkerMessage::SessionStateUpload { data }) => {
                                let bytes = data.len();
                                info!(%session_id, bytes, "received early SessionStateUpload");
                                if let Err(err) = state
                                    .store
                                    .store_session_state_with_actor(
                                        &session_id,
                                        data,
                                        actor_ref.clone(),
                                    )
                                    .await
                                {
                                    warn!(%session_id, bytes, error = %err, "store session_state failed (early)");
                                }
                            }
                            Ok(other) => {
                                error!(%session_id, ?other, "unexpected WorkerMessage in Phase 1/2");
                                let _ = ws_sender.send(Message::Close(None)).await;
                                cleanup(&state, conversation_id.as_ref());
                                return;
                            }
                            Err(err) => {
                                warn!(%session_id, error = %err, "invalid worker message in Phase 1/2; ignoring");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        info!(%session_id, "WebSocket closed before Phase 3");
                        cleanup(&state, conversation_id.as_ref());
                        return;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        error!(%session_id, error = %err, "WebSocket error before Phase 3");
                        cleanup(&state, conversation_id.as_ref());
                        return;
                    }
                }
            }
            // While waiting for Ready, any inbound user message from the
            // relay should also be queued onto drained_user_messages so
            // we can fold the first one into FirstMessage at Ready time.
            forward = to_worker_rx.recv() => {
                if let Some(msg) = forward {
                    if let ServerMessage::Event {
                        event: SessionEvent::UserMessage { content, .. }, ..
                    } = &msg {
                        drained_user_messages.push(content.clone());
                    }
                    if !send_json(&mut ws_sender, &msg).await {
                        cleanup(&state, conversation_id.as_ref());
                        return;
                    }
                }
            }
        }
    }

    pump_phase3(
        ws_sender,
        ws_receiver,
        state,
        session_id,
        conversation_id,
        actor_ref,
        to_worker_rx,
    )
    .await;
}

async fn handle_reconnecting_path(
    mut ws_sender: SplitSink<WebSocket, Message>,
    ws_receiver: futures::stream::SplitStream<WebSocket>,
    state: AppState,
    session_id: SessionId,
    session: crate::domain::sessions::Session,
    last_received_session_event_index: Option<usize>,
    actor_ref: ActorRef,
) {
    let events =
        match collect_session_events_after(&state, &session_id, last_received_session_event_index)
            .await
        {
            Ok(events) => events,
            Err(err) => {
                error!(%session_id, error = %err, "Reconnecting catch-up load failed");
                let _ = ws_sender.send(Message::Close(None)).await;
                return;
            }
        };
    let catch_up = ServerMessage::CatchUp { events };
    if !send_json(&mut ws_sender, &catch_up).await {
        return;
    }

    let conversation_id = session.conversation_id().cloned();
    let (to_worker, to_worker_rx) = mpsc::channel::<ServerMessage>(TO_WORKER_CAPACITY);
    if let Some(conv_id) = conversation_id.as_ref() {
        let drained = state
            .chat_relay_map
            .set_active(conv_id.clone(), session_id.clone(), to_worker, &state.store)
            .await;
        for (event, event_index) in drained {
            let msg = ServerMessage::Event { event, event_index };
            if !send_json(&mut ws_sender, &msg).await {
                cleanup(&state, conversation_id.as_ref());
                return;
            }
        }
    }

    pump_phase3(
        ws_sender,
        ws_receiver,
        state,
        session_id,
        conversation_id,
        actor_ref,
        to_worker_rx,
    )
    .await;
}

/// Phase-2 first-message resolution. Returns `false` if the connection
/// should be torn down.
async fn handle_ready(
    ws_sender: &mut SplitSink<WebSocket, Message>,
    state: &AppState,
    session_id: &SessionId,
    session: &crate::domain::sessions::Session,
    queued_first_user_message: Option<String>,
) -> bool {
    match &session.mode {
        SessionMode::Headless => {
            let agent_prompt = session
                .agent_config
                .system_prompt
                .clone()
                .unwrap_or_default();
            let first = ServerMessage::FirstMessage {
                agent_prompt,
                user_message: String::new(),
            };
            send_json(ws_sender, &first).await
        }
        SessionMode::Interactive {
            conversation_id, ..
        } => {
            let agent_prompt = if session.resumed_from.is_some() {
                String::new()
            } else {
                session
                    .agent_config
                    .system_prompt
                    .clone()
                    .unwrap_or_default()
            };

            if session.mode.greet_user() {
                let first = ServerMessage::FirstMessage {
                    agent_prompt,
                    user_message: String::new(),
                };
                return send_json(ws_sender, &first).await;
            }

            // If a UserMessage was already drained during the Phase-1
            // wait, fold it in now.
            if let Some(content) = queued_first_user_message {
                let first = ServerMessage::FirstMessage {
                    agent_prompt,
                    user_message: content,
                };
                return send_json(ws_sender, &first).await;
            }

            // Try to find the first UserMessage for this conversation in
            // the session log; if absent, stash and return.
            match find_first_user_message_for_conversation(state, conversation_id, session_id).await
            {
                Ok(Some(content)) => {
                    let first = ServerMessage::FirstMessage {
                        agent_prompt,
                        user_message: content,
                    };
                    send_json(ws_sender, &first).await
                }
                Ok(None) => {
                    let stashed = state
                        .chat_relay_map
                        .set_pending_first_message(conversation_id, agent_prompt);
                    if !stashed {
                        warn!(%session_id, "could not stash pending_first_message; bailing");
                        return false;
                    }
                    true
                }
                Err(err) => {
                    error!(%session_id, error = %err, "failed to scan first user message");
                    false
                }
            }
        }
    }
}

/// Phase 3 — bidirectional pump. Worker events go to the session log
/// (and lifecycle events mirror onto the conversation log); inbound
/// `ServerMessage`s from the relay's `to_worker` channel are forwarded
/// to the WS.
async fn pump_phase3(
    mut ws_sender: SplitSink<WebSocket, Message>,
    mut ws_receiver: futures::stream::SplitStream<WebSocket>,
    state: AppState,
    session_id: SessionId,
    conversation_id: Option<ConversationId>,
    actor_ref: ActorRef,
    mut to_worker_rx: mpsc::Receiver<ServerMessage>,
) {
    loop {
        tokio::select! {
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WorkerMessage>(&text) {
                            Ok(WorkerMessage::Event { event }) => {
                                if let Err(err) = handle_worker_event(
                                    &state,
                                    conversation_id.as_ref(),
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
                                    bytes,
                                    "received SessionStateUpload — storing"
                                );
                                match state
                                    .store
                                    .store_session_state_with_actor(
                                        &session_id,
                                        data,
                                        actor_ref.clone(),
                                    )
                                    .await
                                {
                                    Ok(()) => {
                                        info!(%session_id, bytes, "session_state stored");
                                    }
                                    Err(err) => {
                                        warn!(%session_id, bytes, error = %err, "store session_state failed");
                                    }
                                }
                            }
                            Ok(other) => {
                                warn!(%session_id, ?other, "unexpected WorkerMessage in Phase 3");
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
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        error!(%session_id, error = %err, "WebSocket error in Phase 3");
                        break;
                    }
                }
            }
            forward = to_worker_rx.recv() => {
                match forward {
                    Some(msg) => {
                        if !send_json(&mut ws_sender, &msg).await {
                            break;
                        }
                    }
                    None => {
                        info!(%session_id, "to_worker channel closed");
                        break;
                    }
                }
            }
        }
    }

    cleanup(&state, conversation_id.as_ref());
}

fn cleanup(state: &AppState, conversation_id: Option<&ConversationId>) {
    if let Some(conv_id) = conversation_id {
        state.chat_relay_map.disconnect(conv_id);
        info!(%conv_id, "relay unregistered");
    }
}

async fn send_json<S>(ws_sender: &mut S, msg: &ServerMessage) -> bool
where
    S: futures::sink::Sink<Message, Error = axum::Error> + Unpin,
{
    let json = match serde_json::to_string(msg) {
        Ok(s) => s,
        Err(err) => {
            error!(error = %err, "failed to serialize ServerMessage");
            return false;
        }
    };
    if ws_sender.send(Message::Text(json)).await.is_err() {
        warn!("failed to forward ServerMessage; WebSocket closed");
        return false;
    }
    true
}

/// Collect every `SessionEvent` along the prior session's resumption
/// chain, ordered by chain position (oldest first) and then by
/// per-session event index. The worker uses this as primer text when
/// native resume materialization failed.
async fn collect_transcript(
    state: &AppState,
    prior_session_id: &SessionId,
) -> Result<Vec<SessionEvent>, StoreError> {
    let mut chain: Vec<SessionId> = Vec::new();
    let mut cur = Some(prior_session_id.clone());
    while let Some(sid) = cur {
        let session = state.store.get_session(&sid, false).await?.item;
        cur = session.resumed_from.clone();
        chain.push(sid);
    }
    chain.reverse();

    let mut all_events = Vec::new();
    for sid in chain {
        let events = state.store.get_session_events(&sid).await?;
        for v in events {
            all_events.push(v.item.into());
        }
    }
    Ok(all_events)
}

async fn collect_session_events_after(
    state: &AppState,
    session_id: &SessionId,
    last_index: Option<usize>,
) -> Result<Vec<CatchUpEvent>, StoreError> {
    // Per the wire contract, `event_index` is the per-session
    // `VersionNumber` (1-based, monotonic) returned by
    // `append_session_event`. A `last_index = Some(N)` means the worker
    // has seen events up to and including index N; the server returns
    // events with index > N. `None` returns the whole log.
    let threshold = last_index.unwrap_or(0);
    let events = state.store.get_session_events(session_id).await?;
    Ok(events
        .into_iter()
        .filter_map(|v| {
            let event_index = v.version as usize;
            if event_index > threshold {
                Some(CatchUpEvent {
                    event: v.item.into(),
                    event_index,
                })
            } else {
                None
            }
        })
        .collect())
}

/// Find the first `UserMessage` from the conversation's session log
/// (across every prior session linked to the same conversation) so
/// `handle_ready` can fold it into the worker's first turn. Returns
/// `Ok(None)` if no user message exists yet.
async fn find_first_user_message_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
    current_session_id: &SessionId,
) -> Result<Option<String>, StoreError> {
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    let mut query = SearchSessionsQuery::default();
    query.conversation_id = Some(conversation_id.clone());
    let mut sessions = state.store.list_sessions(&query).await?;
    sessions.sort_by_key(|(_, v)| v.creation_time);

    for (sid, _) in sessions {
        let events = state.store.get_session_events(&sid).await?;
        for v in events {
            if let crate::domain::sessions::SessionEvent::UserMessage { content, .. } = v.item {
                return Ok(Some(content));
            }
        }
        if sid == *current_session_id {
            // No earlier session had a user message and we've reached
            // the current one with none seen — return None to stash.
            return Ok(None);
        }
    }
    Ok(None)
}

/// Handle a session event sent by the worker.
///
/// The `Suspending` event is recorded on the session log (worker's record of
/// why it suspended) but does NOT mutate the conversation's status here. The
/// worker exiting after a Suspending event lets the job engine drive the
/// session to `Complete` / `Failed`; `SpawnConversationSessionsAutomation`
/// then flips the conversation `Active → Idle` from that terminal transition.
///
/// `Suspending` and `Closed` ARE additionally mirrored onto the conversation
/// events log (lifecycle history); chat content (`UserMessage` /
/// `AssistantMessage`) lives only on the session log per Phase E step 18.
async fn handle_worker_event(
    state: &AppState,
    conversation_id: Option<&ConversationId>,
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
    // events log for interactive sessions.
    if let Some(conv_id) = conversation_id {
        if let Some(conv_event) = session_event_to_lifecycle_conversation_event(&domain_event) {
            let _ = state
                .store
                .append_conversation_event_with_actor(conv_id, conv_event, actor_ref.clone())
                .await;
        }
    }
    Ok(())
}

/// Map a worker-emitted [`SessionEvent`] onto the corresponding lifecycle
/// [`crate::domain::conversations::ConversationEvent`], if any.
fn session_event_to_lifecycle_conversation_event(
    event: &crate::domain::sessions::SessionEvent,
) -> Option<crate::domain::conversations::ConversationEvent> {
    use crate::domain::conversations::ConversationEvent as ConvEvent;
    use crate::domain::sessions::SessionEvent as SEvent;
    match event {
        SEvent::Suspending { reason, timestamp } => Some(ConvEvent::Suspending {
            reason: reason.clone(),
            timestamp: *timestamp,
        }),
        SEvent::Closed { timestamp } => Some(ConvEvent::Closed {
            timestamp: *timestamp,
        }),
        _ => None,
    }
}
