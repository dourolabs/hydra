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
use hydra_common::api::v1::relay::{CatchUpEvent, ServerMessage, WorkerMessage};
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
    info!(
        session_id = %session_id,
        upgrade = ws.is_some(),
        "session_events_or_relay invoked"
    );
    // Completion logs live on the inner handlers (`session_relay` /
    // `session_events_json`); this dispatcher's only decision is the
    // upgrade branch logged above.
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
    info!(session_id = %session_id, "session_events_json invoked");
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
    info!(
        session_id = %session_id,
        returned = api_events.len(),
        "session_events_json completed"
    );
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

    info!(
        session_id = %session_id,
        "session_relay completed: upgrading to websocket"
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
    // Reply ResumeContext from the session_state store. The blob is keyed
    // on the PRIOR session id — `session_state` is uploaded by a session
    // before it dies, and the successor (this one) inherits it via
    // `resumed_from`. A session with no `resumed_from` has no prior state
    // to resume from.
    let resume_blob = if let Some(prior_id) = session.resumed_from.as_ref() {
        match state.store.get_session_state(prior_id).await {
            Ok(blob) => blob,
            Err(err) => {
                warn!(%session_id, %prior_id, error = %err, "failed to load session_state for ResumeContext");
                None
            }
        }
    } else {
        None
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
    // The entry starts in the `Negotiating` phase, so drained pending
    // events and any concurrent `send_event_to_conversation` arrivals
    // are HELD in the relay buffer until `mark_ready` below — this is
    // what prevents the worker (which strictly expects `Transcript` /
    // `FirstMessage` during Phase 1/2) from seeing pre-Phase-1 `Event`
    // pushes.
    let (to_worker, to_worker_rx) = mpsc::channel::<ServerMessage>(TO_WORKER_CAPACITY);
    let mut drained_pending: Vec<(SessionEvent, usize)> = Vec::new();
    if let Some(conv_id) = conversation_id.as_ref() {
        drained_pending = state
            .chat_relay_map
            .set_active(conv_id.clone(), session_id.clone(), to_worker, &state.store)
            .await;
    }

    // Collect drained UserMessage contents so handle_ready can fold the
    // first one into `FirstMessage`. The Event messages themselves live
    // in the relay's Negotiating buffer and will be flushed by
    // `mark_ready` once handle_ready has sent `FirstMessage`.
    let drained_user_messages: Vec<String> = drained_pending
        .iter()
        .filter_map(|(event, _)| match event {
            SessionEvent::UserMessage { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();

    // Phase-1/Phase-2 loop: handle RequestTranscript fallback and Ready.
    // We deliberately do NOT `select!` on `to_worker_rx` here — while
    // in Negotiating, the chat_relay buffers events instead of pushing
    // through `to_worker`, so nothing should arrive on the rx during
    // this loop.
    loop {
        match ws_receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<WorkerMessage>(&text) {
                    Ok(WorkerMessage::RequestTranscript { prior_session_id }) => {
                        let events = match collect_transcript(&state, &prior_session_id).await {
                            Ok(events) => events,
                            Err(err) => {
                                error!(%session_id, error = %err, "transcript load failed");
                                let _ = ws_sender.send(Message::Close(None)).await;
                                state.disconnect_chat_relay(conversation_id.as_ref());
                                return;
                            }
                        };
                        // Read directly from `session.agent_config.system_prompt`
                        // — same source as `handle_ready`'s `FirstMessage.agent_prompt`
                        // arms. `handle_ready` blanks the prompt on `FirstMessage`
                        // for resumed sessions, so without this field the
                        // transcript-resume path would never deliver the
                        // prompt to the model.
                        let agent_prompt = session.agent_config.system_prompt.clone();
                        let transcript = ServerMessage::Transcript {
                            events,
                            agent_prompt,
                        };
                        if !send_json(&mut ws_sender, &transcript).await {
                            state.disconnect_chat_relay(conversation_id.as_ref());
                            return;
                        }
                    }
                    Ok(WorkerMessage::Ready) => {
                        let outcome = handle_ready(
                            &mut ws_sender,
                            &state,
                            &session_id,
                            &session,
                            drained_user_messages.first().cloned(),
                        )
                        .await;
                        match outcome {
                            ReadyOutcome::FirstMessageSent {
                                folded_user_message,
                            } => {
                                // Phase 1/2 is done from our side. Flush any
                                // buffered events through the WS now — they
                                // come after FirstMessage in worker order.
                                if let Some(conv_id) = conversation_id.as_ref() {
                                    let buffered = state
                                        .chat_relay_map
                                        .mark_ready(conv_id, folded_user_message.as_deref());
                                    for msg in buffered {
                                        if !send_json(&mut ws_sender, &msg).await {
                                            state.disconnect_chat_relay(conversation_id.as_ref());
                                            return;
                                        }
                                    }
                                }
                                break;
                            }
                            ReadyOutcome::Stashed => {
                                // No FirstMessage sent yet — wait for a
                                // user POST to trigger the fold. The
                                // chat_relay's send_event_to_conversation
                                // path handles the transition when that
                                // happens.
                                break;
                            }
                            ReadyOutcome::Failed => {
                                state.disconnect_chat_relay(conversation_id.as_ref());
                                return;
                            }
                        }
                    }
                    // Phase-3 variants arriving before Ready are
                    // tolerated; the worker may begin emitting
                    // events as soon as Phase 1 completes.
                    Ok(WorkerMessage::Event { event }) => {
                        if let Err(err) =
                            handle_worker_event(&state, &session_id, &actor_ref, event).await
                        {
                            error!(%session_id, error = %err, "failed to handle worker event in Phase 1/2");
                        }
                    }
                    Ok(WorkerMessage::SessionStateUpload { data }) => {
                        let bytes = data.len();
                        info!(%session_id, bytes, "received early SessionStateUpload");
                        if let Err(err) = state
                            .store
                            .store_session_state_with_actor(&session_id, data, actor_ref.clone())
                            .await
                        {
                            warn!(%session_id, bytes, error = %err, "store session_state failed (early)");
                        }
                    }
                    Ok(other) => {
                        error!(%session_id, ?other, "unexpected WorkerMessage in Phase 1/2");
                        let _ = ws_sender.send(Message::Close(None)).await;
                        state.disconnect_chat_relay(conversation_id.as_ref());
                        return;
                    }
                    Err(err) => {
                        warn!(%session_id, error = %err, "invalid worker message in Phase 1/2; ignoring");
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                info!(%session_id, "WebSocket closed before Phase 3");
                state.disconnect_chat_relay(conversation_id.as_ref());
                return;
            }
            Some(Ok(Message::Ping(data))) => {
                let _ = ws_sender.send(Message::Pong(data)).await;
            }
            Some(Ok(_)) => {}
            Some(Err(err)) => {
                error!(%session_id, error = %err, "WebSocket error before Phase 3");
                state.disconnect_chat_relay(conversation_id.as_ref());
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
        let _drained = state
            .chat_relay_map
            .set_active(conv_id.clone(), session_id.clone(), to_worker, &state.store)
            .await;
        // For the Reconnecting path, `CatchUp` is the phase-1 completing
        // message — the worker is already past its initial `Fresh` /
        // `RequestTranscript` negotiation and just needs the missed
        // events. Flush the relay buffer (which set_active populated
        // with the drained pending items) immediately, in order. No
        // dedup needed because reconnect doesn't fold into FirstMessage.
        let buffered = state.chat_relay_map.mark_ready(conv_id, None);
        for msg in buffered {
            if !send_json(&mut ws_sender, &msg).await {
                state.disconnect_chat_relay(conversation_id.as_ref());
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

/// Outcome of `handle_ready` for the relay route to dispatch on.
///
/// - `FirstMessageSent` — the server sent `FirstMessage` on the wire;
///   the route must call `mark_ready` to flush any buffered relay
///   events. `folded_user_message` is `Some(content)` iff the server
///   used `content` as `FirstMessage.user_message`; the relay buffer's
///   first matching `UserMessage` Event (if any) is then removed from
///   the flush.
/// - `Stashed` — the server set `pending_first_message` on the relay
///   entry and is waiting for the next user POST to complete phase-1;
///   the relay's `send_event_to_conversation` will handle the fold and
///   flush when that POST arrives.
/// - `Failed` — the connection should be torn down.
enum ReadyOutcome {
    FirstMessageSent { folded_user_message: Option<String> },
    Stashed,
    Failed,
}

/// Phase-2 first-message resolution.
async fn handle_ready(
    ws_sender: &mut SplitSink<WebSocket, Message>,
    state: &AppState,
    session_id: &SessionId,
    session: &crate::domain::sessions::Session,
    queued_first_user_message: Option<String>,
) -> ReadyOutcome {
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
            if send_json(ws_sender, &first).await {
                ReadyOutcome::FirstMessageSent {
                    folded_user_message: None,
                }
            } else {
                ReadyOutcome::Failed
            }
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
                return if send_json(ws_sender, &first).await {
                    ReadyOutcome::FirstMessageSent {
                        folded_user_message: None,
                    }
                } else {
                    ReadyOutcome::Failed
                };
            }

            // If a UserMessage was already drained during the Phase-1
            // wait, fold it in now.
            if let Some(content) = queued_first_user_message {
                let first = ServerMessage::FirstMessage {
                    agent_prompt,
                    user_message: content.clone(),
                };
                return if send_json(ws_sender, &first).await {
                    ReadyOutcome::FirstMessageSent {
                        folded_user_message: Some(content),
                    }
                } else {
                    ReadyOutcome::Failed
                };
            }

            // Try to find the first UserMessage for this conversation in
            // the session log; if absent, stash and return.
            match find_first_user_message_for_conversation(state, conversation_id, session_id).await
            {
                Ok(Some(content)) => {
                    let first = ServerMessage::FirstMessage {
                        agent_prompt,
                        user_message: content.clone(),
                    };
                    if send_json(ws_sender, &first).await {
                        // The store scan may return a UserMessage from a
                        // *prior* session (e.g. on resume); the relay
                        // buffer only holds events for the current
                        // session. `mark_ready` only dedupes when the
                        // buffer actually contains an Event with this
                        // content, so it's safe to always pass it.
                        ReadyOutcome::FirstMessageSent {
                            folded_user_message: Some(content),
                        }
                    } else {
                        ReadyOutcome::Failed
                    }
                }
                Ok(None) => {
                    let stashed = state
                        .chat_relay_map
                        .set_pending_first_message(conversation_id, agent_prompt);
                    if !stashed {
                        warn!(%session_id, "could not stash pending_first_message; bailing");
                        return ReadyOutcome::Failed;
                    }
                    ReadyOutcome::Stashed
                }
                Err(err) => {
                    error!(%session_id, error = %err, "failed to scan first user message");
                    ReadyOutcome::Failed
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
                                if let Err(err) =
                                    handle_worker_event(&state, &session_id, &actor_ref, event)
                                        .await
                                {
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

    state.disconnect_chat_relay(conversation_id.as_ref());
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
/// Chat content (`UserMessage` / `AssistantMessage`) lives only on the
/// session log per Phase E step 18. The conversation's own status sequence
/// (each transition is a new versioned row on `conversations` /
/// `conversations_v2`) carries the lifecycle history.
async fn handle_worker_event(
    state: &AppState,
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
        .append_session_event_with_actor(session_id, domain_event, actor_ref.clone())
        .await?;
    Ok(())
}
