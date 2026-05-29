use dashmap::DashMap;
use hydra_common::api::v1::sessions::{SearchSessionsQuery, SessionEvent as ApiSessionEvent};
use hydra_common::api::v1::task_status::Status;
use hydra_common::{ConversationId, SessionId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::app::app_state::AppState;
use crate::domain::actors::ActorRef;
use crate::domain::conversations::ConversationEvent as DomainConversationEvent;
use crate::domain::sessions::SessionEvent;
use crate::store::StoreError;

/// A relay entry associated with an active conversation. Holds the channel
/// for sending user messages to a connected worker, plus the session id of
/// the worker currently relaying the conversation (used for kill_job, etc.).
#[derive(Debug, Clone)]
pub struct RelayEntry {
    /// The session id of the worker currently connected to this conversation.
    pub session_id: SessionId,
    /// Send session events (currently only `UserMessage`) TO the worker
    /// (server -> worker direction).
    pub to_worker: mpsc::Sender<ApiSessionEvent>,
}

/// In-memory map of active conversation relays. Maps conversation IDs to
/// their relay entries, enabling the server to route messages between
/// frontends and worker containers.
pub type ChatRelayMap = Arc<DashMap<ConversationId, RelayEntry>>;

/// Channel capacity for the server->worker mpsc channel.
const TO_WORKER_CAPACITY: usize = 64;

/// Register a new relay for the given conversation. Returns the receiving end
/// of the server->worker channel so the WebSocket handler can use it.
pub fn register_relay(
    relay_map: &ChatRelayMap,
    conversation_id: ConversationId,
    session_id: SessionId,
) -> mpsc::Receiver<ApiSessionEvent> {
    let (to_worker_tx, to_worker_rx) = mpsc::channel(TO_WORKER_CAPACITY);

    let entry = RelayEntry {
        session_id,
        to_worker: to_worker_tx,
    };
    relay_map.insert(conversation_id, entry);

    to_worker_rx
}

/// Unregister the relay for the given conversation, cleaning up channels.
pub fn unregister_relay(relay_map: &ChatRelayMap, conversation_id: &ConversationId) {
    relay_map.remove(conversation_id);
}

/// Send a session event to the worker for the given conversation.
/// Returns an error if the conversation has no active relay or the channel is full.
pub async fn send_to_worker(
    relay_map: &ChatRelayMap,
    conversation_id: &ConversationId,
    event: ApiSessionEvent,
) -> Result<(), SendToWorkerError> {
    let entry = relay_map
        .get(conversation_id)
        .ok_or(SendToWorkerError::NoRelay)?;
    entry
        .to_worker
        .send(event)
        .await
        .map_err(|_| SendToWorkerError::ChannelClosed)
}

#[derive(Debug, thiserror::Error)]
pub enum SendToWorkerError {
    #[error("no active relay for conversation")]
    NoRelay,
    #[error("relay channel closed")]
    ChannelClosed,
}

/// Resolve the *active* session that "owns" the given conversation right now.
///
/// Used by `send_message` and the lifecycle write paths to find the session
/// id to attach a `SessionEvent` to when the caller only has a
/// `ConversationId` in hand. Prefers the in-memory `chat_relay_map` (set by
/// a live worker WebSocket — a live relay entry by definition tracks an
/// active worker); falls back to a store query filtered to active session
/// states (`Created` / `Pending` / `Running`) and picks the most-recently
/// created one. Terminated sessions are never returned.
///
/// Returns `None` if no active session is currently linked to the
/// conversation — callers needing to wait briefly for one to appear should
/// use [`wait_for_active_session_for_conversation`] instead.
pub async fn resolve_session_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
) -> Option<SessionId> {
    if let Some(entry) = state.chat_relay_map.get(conversation_id) {
        return Some(entry.session_id.clone());
    }
    let mut query = SearchSessionsQuery::default();
    query.conversation_id = Some(conversation_id.clone());
    query.status = vec![Status::Created, Status::Pending, Status::Running];
    let sessions = state.store().list_sessions(&query).await.ok()?;
    sessions
        .into_iter()
        .max_by_key(|(_, v)| v.creation_time)
        .map(|(id, _)| id)
}

/// Bounded-wait variant of [`resolve_session_for_conversation`].
///
/// Polls the resolver for an active session for up to ~2s (20 × 100ms).
/// Used by the chat-content write path (`send_message`) on a brand-new
/// conversation or right after a resume, where
/// `SpawnConversationSessionsAutomation` is spawning the companion session
/// concurrently and may not have produced it yet when `send_message`
/// lands. On timeout returns
/// [`ResolveActiveSessionError::Timeout`] so the caller surfaces a
/// non-200 to the client rather than silently dropping the user message.
pub async fn wait_for_active_session_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
) -> Result<SessionId, ResolveActiveSessionError> {
    const RETRIES: u32 = 20;
    const DELAY_MS: u64 = 100;
    for _ in 0..RETRIES {
        if let Some(id) = resolve_session_for_conversation(state, conversation_id).await {
            return Ok(id);
        }
        tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
    }
    resolve_session_for_conversation(state, conversation_id)
        .await
        .ok_or_else(|| ResolveActiveSessionError::Timeout {
            conversation_id: conversation_id.clone(),
        })
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveActiveSessionError {
    #[error("no active session for conversation '{conversation_id}' after wait budget")]
    Timeout { conversation_id: ConversationId },
}

/// Resolve the most-recently-created session linked to a conversation,
/// **without** the active-state filter applied by
/// [`resolve_session_for_conversation`].
///
/// Used by the dual-write helpers below: lifecycle events (`Suspending`,
/// `Closed`) are routinely appended *after* the worker session has gone
/// terminal, so the dual-write needs to find the now-terminal session to
/// land the event on its log. The chat-content write path
/// (`send_message`) uses the filtered variant instead — it must never
/// write a `UserMessage` to a dead session.
///
/// This helper is scoped to the dual-write callers (part 3 of
/// [[i-piwnljcg]] inlines them); new callers should prefer
/// [`resolve_session_for_conversation`] unless they have an explicit need
/// to address terminated sessions.
async fn resolve_session_for_conversation_any_status(
    state: &AppState,
    conversation_id: &ConversationId,
) -> Option<SessionId> {
    if let Some(entry) = state.chat_relay_map.get(conversation_id) {
        return Some(entry.session_id.clone());
    }
    let mut query = SearchSessionsQuery::default();
    query.conversation_id = Some(conversation_id.clone());
    let sessions = state.store().list_sessions(&query).await.ok()?;
    sessions
        .into_iter()
        .max_by_key(|(_, v)| v.creation_time)
        .map(|(id, _)| id)
}

/// Dual-write a `SessionEvent` to the session backing this conversation.
///
/// Logs a warn and returns `Ok(())` if no session exists yet (this happens
/// for a brief window between a conversation `Idle→Active` flip and the
/// `SpawnConversationSessionsAutomation` creating the new session). The
/// matching `ConversationEvent` write is the source of truth during the
/// dual-write phase, so skipping the `SessionEvent` here is non-fatal.
pub async fn dual_write_session_event_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
    event: SessionEvent,
    actor: ActorRef,
) -> Result<(), StoreError> {
    let Some(session_id) =
        resolve_session_for_conversation_any_status(state, conversation_id).await
    else {
        warn!(
            %conversation_id,
            "dual-write SessionEvent skipped: no session linked to conversation yet"
        );
        return Ok(());
    };
    dual_write_session_event(state, &session_id, event, actor).await
}

/// Dual-write a `SessionEvent` against a known session id.
///
/// Errors during dual-write are NOT propagated to the caller — the matching
/// `ConversationEvent` write is the source of truth in the dual-write phase
/// and the SessionEvent log is a follow-along sink that observability will
/// surface independently. We log at warn level so any drift is visible.
pub async fn dual_write_session_event(
    state: &AppState,
    session_id: &SessionId,
    event: SessionEvent,
    actor: ActorRef,
) -> Result<(), StoreError> {
    let preview = event.preview();
    match state
        .store
        .append_session_event_with_actor(session_id, event, actor)
        .await
    {
        Ok(version) => {
            info!(
                %session_id,
                version,
                event = %preview,
                "dual-write SessionEvent appended",
            );
            Ok(())
        }
        Err(err) => {
            warn!(
                %session_id,
                event = %preview,
                error = %err,
                "dual-write SessionEvent failed",
            );
            Ok(())
        }
    }
}

/// Map a lifecycle [`DomainConversationEvent`] to its [`SessionEvent`] twin
/// per design §3.2. Used by `close_conversation` (and other lifecycle write
/// paths) to dual-write the matching session event alongside the legacy
/// conversation event.
///
/// `Resumed` is mapped to `None` here because the producing session id is the
/// new session and the prior `from_session_id` only exists in the automation
/// that created the new session — the automation writes the SessionEvent
/// directly, not via this mapping.
///
/// Chat-content variants (`UserMessage`, `AssistantMessage`) no longer live
/// on `ConversationEvent` after Phase E step 18; the worker emits them as
/// `SessionEvent` directly and `send_message` writes them as `SessionEvent`,
/// so they don't appear in this mapping.
pub fn conversation_event_to_session_event(
    event: &DomainConversationEvent,
) -> Option<SessionEvent> {
    match event {
        DomainConversationEvent::Suspending { reason, timestamp } => {
            Some(SessionEvent::Suspending {
                reason: reason.clone(),
                timestamp: *timestamp,
            })
        }
        DomainConversationEvent::Closed { timestamp } => Some(SessionEvent::Closed {
            timestamp: *timestamp,
        }),
        // Resumed has different semantics on the two logs: the
        // ConversationEvent carries the *new* session id, while
        // SessionEvent::Resumed records the *prior* (from) session id on the
        // new session. The dual-write for Resumed is performed inside the
        // `SpawnConversationSessionsAutomation`, which has both ids.
        DomainConversationEvent::Resumed { .. } => None,
    }
}

/// Persist a `session_state` blob against a known session id. Errors are logged
/// and swallowed so a transient store failure does not tear down the WebSocket
/// relay; the worker will retry the upload on its next Suspending event.
pub async fn store_session_state(
    state: &AppState,
    session_id: &SessionId,
    data: Vec<u8>,
    actor: ActorRef,
) -> Result<(), StoreError> {
    let bytes = data.len();
    match state
        .store
        .store_session_state_with_actor(session_id, data, actor)
        .await
    {
        Ok(()) => {
            info!(
                %session_id,
                bytes,
                "session_state stored",
            );
            Ok(())
        }
        Err(err) => {
            warn!(
                %session_id,
                bytes,
                error = %err,
                "store session_state failed",
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_helpers::state_with_default_model;
    use crate::domain::sessions::{AgentConfig, Session, SessionMode};
    use crate::domain::task_status::Status as DomainStatus;
    use crate::domain::users::Username;
    use crate::routes::sessions::mount_spec_from_create_request;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::time::Instant;

    fn test_relay_map() -> ChatRelayMap {
        Arc::new(DashMap::new())
    }

    fn interactive_session(conversation_id: &ConversationId, status: DomainStatus) -> Session {
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Interactive {
                conversation_id: conversation_id.clone(),
                idle_timeout_secs: None,
                conversation_resume_from: None,
            },
            status,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn register_and_unregister_relay() {
        let map = test_relay_map();
        let conversation_id = ConversationId::new();
        let session_id = SessionId::new();

        let _rx = register_relay(&map, conversation_id.clone(), session_id.clone());
        assert!(map.contains_key(&conversation_id));
        assert_eq!(map.get(&conversation_id).unwrap().session_id, session_id);

        unregister_relay(&map, &conversation_id);
        assert!(!map.contains_key(&conversation_id));
    }

    #[tokio::test]
    async fn send_to_worker_delivers_message() {
        let map = test_relay_map();
        let conversation_id = ConversationId::new();
        let session_id = SessionId::new();
        let mut rx = register_relay(&map, conversation_id.clone(), session_id);

        let event = ApiSessionEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        send_to_worker(&map, &conversation_id, event.clone())
            .await
            .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn send_to_worker_no_relay_returns_error() {
        let map = test_relay_map();
        let conversation_id = ConversationId::new();

        let event = ApiSessionEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        let result = send_to_worker(&map, &conversation_id, event).await;
        assert!(matches!(result, Err(SendToWorkerError::NoRelay)));
    }

    #[tokio::test]
    async fn resolve_session_for_conversation_filters_terminated_sessions() {
        // A session that has already transitioned to a terminal status
        // (`Complete` / `Failed`) must not be returned by the resolver —
        // the active-state filter is what lets the caller treat the
        // resolver result as "currently owns the conversation".
        let state = state_with_default_model("default-model");
        let conversation_id = ConversationId::new();

        for status in [DomainStatus::Complete, DomainStatus::Failed] {
            let session = interactive_session(&conversation_id, status);
            state
                .store
                .add_session_with_actor(
                    session,
                    Utc::now(),
                    crate::domain::actors::ActorRef::test(),
                )
                .await
                .unwrap();
        }

        let resolved = resolve_session_for_conversation(&state, &conversation_id).await;
        assert!(
            resolved.is_none(),
            "terminated sessions must not be returned by the resolver, got {resolved:?}",
        );
    }

    #[tokio::test]
    async fn resolve_session_for_conversation_returns_active_session() {
        // The happy path: an active session in `Running` exists for this
        // conversation, so the resolver returns it.
        let state = state_with_default_model("default-model");
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), crate::domain::actors::ActorRef::test())
            .await
            .unwrap();

        let resolved = resolve_session_for_conversation(&state, &conversation_id).await;
        assert_eq!(resolved, Some(session_id));
    }

    #[tokio::test]
    async fn wait_for_active_session_times_out_when_no_session_appears() {
        // No session is ever inserted, so the bounded-wait resolver must
        // surface a `Timeout` error after the wait budget elapses. The
        // budget is 20 × 100ms = 2s, so we give ourselves headroom on the
        // upper bound to keep the test stable on slow runners.
        let state = state_with_default_model("default-model");
        let conversation_id = ConversationId::new();

        let started = Instant::now();
        let result = wait_for_active_session_for_conversation(&state, &conversation_id).await;
        let elapsed = started.elapsed();

        match result {
            Err(ResolveActiveSessionError::Timeout {
                conversation_id: id,
            }) => {
                assert_eq!(id, conversation_id);
            }
            other => panic!("expected Timeout error, got {other:?}"),
        }
        assert!(
            elapsed >= std::time::Duration::from_millis(1900),
            "expected the resolver to spend ~2s before timing out, got {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn wait_for_active_session_skips_terminated_and_succeeds_on_active() {
        // A terminated session is present alongside a freshly-active one.
        // The resolver must return the active one (the terminated one is
        // filtered out by the status query, and the active one wins the
        // `max_by_key(creation_time)` tiebreak by virtue of being created
        // later).
        let state = state_with_default_model("default-model");
        let conversation_id = ConversationId::new();

        let terminated = interactive_session(&conversation_id, DomainStatus::Complete);
        state
            .store
            .add_session_with_actor(
                terminated,
                Utc::now(),
                crate::domain::actors::ActorRef::test(),
            )
            .await
            .unwrap();

        let active = interactive_session(&conversation_id, DomainStatus::Running);
        let (active_id, _) = state
            .store
            .add_session_with_actor(
                active,
                Utc::now() + chrono::Duration::milliseconds(1),
                crate::domain::actors::ActorRef::test(),
            )
            .await
            .unwrap();

        let resolved = wait_for_active_session_for_conversation(&state, &conversation_id)
            .await
            .expect("expected the active session to be returned");
        assert_eq!(resolved, active_id);
    }
}
