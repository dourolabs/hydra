use dashmap::DashMap;
use hydra_common::SessionId;
use hydra_common::api::v1::conversations::ConversationEvent;
use std::sync::Arc;
use tokio::sync::mpsc;

/// A relay sender associated with an active session. Holds the channel for
/// sending user messages to a connected worker.
#[derive(Debug, Clone)]
pub struct RelaySender {
    /// Send user messages TO the worker (server -> worker direction).
    pub to_worker: mpsc::Sender<ConversationEvent>,
}

/// In-memory map of active session relays. Maps session IDs to their
/// relay channel senders, enabling the server to route messages between
/// frontends and worker containers.
pub type ChatRelayMap = Arc<DashMap<SessionId, RelaySender>>;

/// Channel capacity for the server->worker mpsc channel.
const TO_WORKER_CAPACITY: usize = 64;

/// Register a new relay for the given session. Returns the receiving end
/// of the server->worker channel so the WebSocket handler can use it.
pub fn register_relay(
    relay_map: &ChatRelayMap,
    session_id: SessionId,
) -> mpsc::Receiver<ConversationEvent> {
    let (to_worker_tx, to_worker_rx) = mpsc::channel(TO_WORKER_CAPACITY);

    let sender = RelaySender {
        to_worker: to_worker_tx,
    };
    relay_map.insert(session_id, sender);

    to_worker_rx
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

        let _rx = register_relay(&map, session_id.clone());
        assert!(map.contains_key(&session_id));

        unregister_relay(&map, &session_id);
        assert!(!map.contains_key(&session_id));
    }

    #[tokio::test]
    async fn send_to_worker_delivers_message() {
        let map = test_relay_map();
        let session_id = SessionId::new();
        let mut rx = register_relay(&map, session_id.clone());

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
}
