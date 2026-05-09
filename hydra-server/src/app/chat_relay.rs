use dashmap::DashMap;
use hydra_common::SessionId;
use hydra_common::api::v1::conversations::ConversationEvent;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// A relay sender associated with an active session. Holds channels for
/// bidirectional communication between the server and a connected worker.
#[derive(Debug, Clone)]
pub struct RelaySender {
    /// Send user messages TO the worker (server -> worker direction).
    pub to_worker: mpsc::Sender<ConversationEvent>,
    /// Broadcast worker messages to listeners (worker -> server direction).
    pub from_worker: broadcast::Sender<ConversationEvent>,
}

/// In-memory map of active session relays. Maps session IDs to their
/// relay channel senders, enabling the server to route messages between
/// frontends and worker containers.
pub type ChatRelayMap = Arc<DashMap<SessionId, RelaySender>>;

/// Channel capacity for the server->worker mpsc channel.
const TO_WORKER_CAPACITY: usize = 64;
/// Channel capacity for the worker->server broadcast channel.
const FROM_WORKER_CAPACITY: usize = 256;

/// Register a new relay for the given session. Returns the receiving ends
/// of both channels so the WebSocket handler can use them.
pub fn register_relay(
    relay_map: &ChatRelayMap,
    session_id: SessionId,
) -> (
    mpsc::Receiver<ConversationEvent>,
    broadcast::Receiver<ConversationEvent>,
) {
    let (to_worker_tx, to_worker_rx) = mpsc::channel(TO_WORKER_CAPACITY);
    let (from_worker_tx, from_worker_rx) = broadcast::channel(FROM_WORKER_CAPACITY);

    let sender = RelaySender {
        to_worker: to_worker_tx,
        from_worker: from_worker_tx,
    };
    relay_map.insert(session_id, sender);

    (to_worker_rx, from_worker_rx)
}

/// Unregister the relay for the given session, cleaning up channels.
pub fn unregister_relay(relay_map: &ChatRelayMap, session_id: &SessionId) {
    relay_map.remove(session_id);
}

/// Send a conversation event to the worker for the given session.
/// Returns an error if the session has no active relay or the channel is full.
pub async fn send_to_worker(
    relay_map: &ChatRelayMap,
    session_id: &SessionId,
    event: ConversationEvent,
) -> Result<(), SendToWorkerError> {
    let entry = relay_map
        .get(session_id)
        .ok_or(SendToWorkerError::NoRelay)?;
    entry
        .to_worker
        .send(event)
        .await
        .map_err(|_| SendToWorkerError::ChannelClosed)
}

/// Get a broadcast receiver for worker messages for the given session.
pub fn subscribe_to_worker(
    relay_map: &ChatRelayMap,
    session_id: &SessionId,
) -> Option<broadcast::Receiver<ConversationEvent>> {
    relay_map
        .get(session_id)
        .map(|entry| entry.from_worker.subscribe())
}

#[derive(Debug, thiserror::Error)]
pub enum SendToWorkerError {
    #[error("no active relay for session")]
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
        let session_id = SessionId::new();

        let (_rx, _broadcast_rx) = register_relay(&map, session_id.clone());
        assert!(map.contains_key(&session_id));

        unregister_relay(&map, &session_id);
        assert!(!map.contains_key(&session_id));
    }

    #[tokio::test]
    async fn send_to_worker_delivers_message() {
        let map = test_relay_map();
        let session_id = SessionId::new();
        let (mut rx, _broadcast_rx) = register_relay(&map, session_id.clone());

        let event = ConversationEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        send_to_worker(&map, &session_id, event.clone())
            .await
            .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn send_to_worker_no_relay_returns_error() {
        let map = test_relay_map();
        let session_id = SessionId::new();

        let event = ConversationEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        let result = send_to_worker(&map, &session_id, event).await;
        assert!(matches!(result, Err(SendToWorkerError::NoRelay)));
    }

    #[tokio::test]
    async fn subscribe_to_worker_receives_broadcasts() {
        let map = test_relay_map();
        let session_id = SessionId::new();
        let (_rx, _broadcast_rx) = register_relay(&map, session_id.clone());

        let mut subscriber = subscribe_to_worker(&map, &session_id).unwrap();

        // Broadcast a message through the from_worker channel
        let entry = map.get(&session_id).unwrap();
        let event = ConversationEvent::AssistantMessage {
            content: "hi there".to_string(),
            timestamp: Utc::now(),
        };
        entry.from_worker.send(event.clone()).unwrap();
        drop(entry);

        let received = subscriber.recv().await.unwrap();
        assert_eq!(received, event);
    }

    #[test]
    fn subscribe_to_worker_no_relay_returns_none() {
        let map = test_relay_map();
        let session_id = SessionId::new();
        assert!(subscribe_to_worker(&map, &session_id).is_none());
    }
}
