use dashmap::DashMap;
use hydra_common::api::v1::sessions::{SearchSessionsQuery, SessionEvent as ApiSessionEvent};
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

/// Resolve the session that "owns" the given conversation right now.
///
/// Used by `send_message` and the lifecycle write paths to find the session
/// id to attach a `SessionEvent` to when the caller only has a
/// `ConversationId` in hand. Prefers the in-memory `chat_relay_map` (set by
/// a live worker WebSocket); falls back to picking the most-recently-created
/// session linked to the conversation. Returns `None` if no session has been
/// spawned yet — the write is best-effort and is skipped (with a warn log at
/// the call site) in that case.
pub async fn resolve_session_for_conversation(
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

/// Like [`resolve_session_for_conversation`] but polls briefly for the
/// session to appear. Used by the chat-content write path
/// (`send_message`) on a brand-new conversation, where
/// `SpawnConversationSessionsAutomation` is spawning the companion session
/// concurrently and may not have produced it yet when `send_message` lands.
///
/// The retry budget is short on purpose: this path is on the user-facing
/// `POST /v1/conversations/:id/messages` (and the immediate-after-create
/// follow-up from `create_conversation`), so we want to bound the worst
/// case. If the session still isn't there after the budget, the caller
/// proceeds without a session-event write and a warn fires.
pub async fn resolve_session_for_conversation_with_retry(
    state: &AppState,
    conversation_id: &ConversationId,
) -> Option<SessionId> {
    const RETRIES: u32 = 20;
    const DELAY_MS: u64 = 100;
    for _ in 0..RETRIES {
        if let Some(id) = resolve_session_for_conversation(state, conversation_id).await {
            return Some(id);
        }
        tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
    }
    resolve_session_for_conversation(state, conversation_id).await
}

/// Like [`resolve_session_for_conversation_with_retry`] but waits for a
/// session strictly *newer* than `prior` to appear. Used by `send_message`
/// in the resume-on-send path so the user message lands on the freshly
/// spawned session rather than the prior (now-closed) one. If `prior` is
/// `None`, falls back to the standard retry resolver.
pub async fn wait_for_new_session_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
    prior: Option<&SessionId>,
) -> Option<SessionId> {
    let Some(prior) = prior else {
        return resolve_session_for_conversation_with_retry(state, conversation_id).await;
    };
    const RETRIES: u32 = 20;
    const DELAY_MS: u64 = 100;
    for _ in 0..RETRIES {
        if let Some(id) = resolve_session_for_conversation(state, conversation_id).await {
            if &id != prior {
                return Some(id);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
    }
    resolve_session_for_conversation(state, conversation_id).await
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
    let Some(session_id) = resolve_session_for_conversation(state, conversation_id).await else {
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

/// Dual-write a `session_state` blob against a known session id.
pub async fn dual_write_session_state(
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
                "dual-write session_state stored",
            );
            Ok(())
        }
        Err(err) => {
            warn!(
                %session_id,
                bytes,
                error = %err,
                "dual-write session_state failed",
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_relay_map() -> ChatRelayMap {
        Arc::new(DashMap::new())
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
}
