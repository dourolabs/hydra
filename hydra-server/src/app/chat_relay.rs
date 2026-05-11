use dashmap::DashMap;
use hydra_common::api::v1::conversations::ConversationEvent;
use hydra_common::{ConversationId, SessionId};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A relay entry associated with an active conversation. Holds the channel
/// for sending user messages to a connected worker, plus the session id of
/// the worker currently relaying the conversation (used for kill_job, etc.).
#[derive(Debug, Clone)]
pub struct RelayEntry {
    /// The session id of the worker currently connected to this conversation.
    pub session_id: SessionId,
    /// Send user messages TO the worker (server -> worker direction).
    pub to_worker: mpsc::Sender<ConversationEvent>,
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
) -> mpsc::Receiver<ConversationEvent> {
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

/// Send a conversation event to the worker for the given conversation.
/// Returns an error if the conversation has no active relay or the channel is full.
pub async fn send_to_worker(
    relay_map: &ChatRelayMap,
    conversation_id: &ConversationId,
    event: ConversationEvent,
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

        let event = ConversationEvent::UserMessage {
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

        let event = ConversationEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        let result = send_to_worker(&map, &conversation_id, event).await;
        assert!(matches!(result, Err(SendToWorkerError::NoRelay)));
    }
}
