use dashmap::DashMap;
use hydra_common::api::v1::sessions::SessionEvent as ApiSessionEvent;
use hydra_common::{ConversationId, SessionId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::app::event_bus::StoreWithEvents;
use crate::domain::actors::ActorRef;
use crate::domain::sessions::SessionEvent;

/// Channel capacity for the server->worker mpsc channel.
pub const TO_WORKER_CAPACITY: usize = 64;

/// In-memory queue/route table for chat-relay traffic between the
/// conversation HTTP routes and connected workers.
///
/// Each conversation is in one of two states, both held as an internal
/// [`Entry`]:
///
/// - [`Entry::ActiveConnection`] — a worker has connected and registered
///   via [`ChatRelayMap::set_active`]. Subsequent events are delivered
///   over the per-conversation mpsc and dual-written to the session log.
/// - [`Entry::PendingConnection`] — no worker has registered yet (either
///   the companion session is still being spawned or the worker has not
///   yet completed its WebSocket handshake). Events accepted here are
///   queued in FIFO order and delivered atomically at the Pending→Active
///   transition.
///
/// The enum and the inner map are private; all access goes through the
/// methods on this struct so callers never pattern-match on the state.
#[derive(Clone)]
pub struct ChatRelayMap {
    inner: Arc<DashMap<ConversationId, Entry>>,
}

enum Entry {
    ActiveConnection {
        session_id: SessionId,
        to_worker: mpsc::Sender<ApiSessionEvent>,
    },
    PendingConnection {
        pending: Vec<PendingItem>,
    },
}

/// A pending event held alongside the actor that originated it so the
/// dual-write at drain time preserves authorship in the session log.
struct PendingItem {
    event: ApiSessionEvent,
    actor: ActorRef,
}

#[derive(Debug, thiserror::Error)]
pub enum SendEventError {
    #[error("relay channel closed for conversation")]
    ChannelClosed,
}

impl ChatRelayMap {
    // Deliberate single explicit constructor; `Default` is omitted to
    // keep the construction surface minimal.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Accept an event for `conversation_id`. If a worker is connected
    /// ([`Entry::ActiveConnection`]) the event is dual-written to the
    /// session log and forwarded to the worker's channel. If no worker
    /// is connected yet, the event is queued
    /// ([`Entry::PendingConnection`]) and will be delivered on the next
    /// [`set_active`](Self::set_active) call.
    ///
    /// The decision (active vs pending) is made under the DashMap entry
    /// lock so a concurrent [`set_active`](Self::set_active) cannot race
    /// a message past the drain.
    pub async fn send_event_to_conversation(
        &self,
        conversation_id: &ConversationId,
        event: ApiSessionEvent,
        store: &StoreWithEvents,
        actor: ActorRef,
    ) -> Result<(), SendEventError> {
        enum Decision {
            Active {
                session_id: SessionId,
                to_worker: mpsc::Sender<ApiSessionEvent>,
            },
            Queued,
        }
        let decision = {
            let mut entry = self
                .inner
                .entry(conversation_id.clone())
                .or_insert_with(|| Entry::PendingConnection {
                    pending: Vec::new(),
                });
            match entry.value_mut() {
                Entry::ActiveConnection {
                    session_id,
                    to_worker,
                } => Decision::Active {
                    session_id: session_id.clone(),
                    to_worker: to_worker.clone(),
                },
                Entry::PendingConnection { pending } => {
                    pending.push(PendingItem {
                        event: event.clone(),
                        actor: actor.clone(),
                    });
                    Decision::Queued
                }
            }
        };

        match decision {
            Decision::Queued => {
                info!(
                    %conversation_id,
                    "no relay connected, event queued until worker connects"
                );
                Ok(())
            }
            Decision::Active {
                session_id,
                to_worker,
            } => {
                dual_write_session_event(store, &session_id, event.clone(), actor).await;
                to_worker
                    .send(event)
                    .await
                    .map_err(|_| SendEventError::ChannelClosed)
            }
        }
    }

    /// Atomically transition the entry for `conversation_id` to
    /// [`Entry::ActiveConnection`] and return any events that were queued
    /// while it was [`Entry::PendingConnection`].
    ///
    /// Each drained event is dual-written to the freshly-connected
    /// session's event log before this call returns, so a subsequent
    /// `Reconnecting` catch-up replays them from the log. The relay
    /// route is then responsible for delivering the returned vec to the
    /// worker over the WebSocket as the first "live" messages, after
    /// the catch-up has been sent.
    ///
    /// If an entry already exists in `ActiveConnection` (e.g. a second
    /// worker connection for the same conversation) the new entry
    /// replaces it; the prior `to_worker` channel is dropped, which
    /// closes the old relay loop. The returned vec is empty in that
    /// case (there were no pending events).
    pub async fn set_active(
        &self,
        conversation_id: ConversationId,
        session_id: SessionId,
        to_worker: mpsc::Sender<ApiSessionEvent>,
        store: &StoreWithEvents,
    ) -> Vec<ApiSessionEvent> {
        let drained: Vec<PendingItem> = {
            let mut entry = self
                .inner
                .entry(conversation_id.clone())
                .or_insert_with(|| Entry::ActiveConnection {
                    session_id: session_id.clone(),
                    to_worker: to_worker.clone(),
                });
            // `or_insert_with` either inserted a fresh ActiveConnection
            // (drained = []) or returned a pre-existing entry that we now
            // overwrite. Use `mem::replace` to swap and capture the prior
            // value atomically under the entry lock.
            let previous = std::mem::replace(
                entry.value_mut(),
                Entry::ActiveConnection {
                    session_id: session_id.clone(),
                    to_worker,
                },
            );
            match previous {
                Entry::PendingConnection { pending } => pending,
                Entry::ActiveConnection { .. } => Vec::new(),
            }
        };

        // The DashMap entry lock is released above before this loop
        // runs, so a concurrent `send_event_to_conversation` that
        // arrives now will (correctly) observe `ActiveConnection` and
        // dual-write its event in parallel. The session log assigns
        // versions in arrival order, so its event may interleave
        // between drained items on the log; live worker order is still
        // preserved because the relay route forwards `drained_events`
        // synchronously before entering the bidirectional loop. A
        // later `Reconnecting` catch-up could replay the log out of
        // strict FIFO with the pending vec — the race window is tiny
        // (same user driving send_message and the worker socket), and
        // strict cross-source FIFO is left to future work.
        let mut drained_events = Vec::with_capacity(drained.len());
        for item in drained {
            dual_write_session_event(store, &session_id, item.event.clone(), item.actor).await;
            drained_events.push(item.event);
        }
        drained_events
    }

    /// Remove the entry entirely. Called when a worker WebSocket
    /// disconnects so a subsequent reconnect starts from a clean slate.
    pub fn disconnect(&self, conversation_id: &ConversationId) {
        self.inner.remove(conversation_id);
    }

    /// Returns the session id of the currently-connected worker, or
    /// `None` if no worker is connected (i.e. no entry, or the entry is
    /// still `PendingConnection`).
    pub fn active_session_id(&self, conversation_id: &ConversationId) -> Option<SessionId> {
        self.inner
            .get(conversation_id)
            .and_then(|entry| match entry.value() {
                Entry::ActiveConnection { session_id, .. } => Some(session_id.clone()),
                Entry::PendingConnection { .. } => None,
            })
    }
}

/// Dual-write an event to the session's event log. Errors are logged
/// and swallowed: the worker's own write of the equivalent event
/// remains the source of truth during the dual-write phase, and a
/// transient store failure must not tear down the relay.
async fn dual_write_session_event(
    store: &StoreWithEvents,
    session_id: &SessionId,
    event: ApiSessionEvent,
    actor: ActorRef,
) {
    let domain_event: SessionEvent = match event.try_into() {
        Ok(e) => e,
        Err(_) => {
            warn!(
                %session_id,
                "dual-write skipped: unknown SessionEvent variant",
            );
            return;
        }
    };
    let preview = domain_event.preview();
    match store
        .append_session_event_with_actor(session_id, domain_event, actor)
        .await
    {
        Ok(version) => {
            info!(
                %session_id,
                version,
                event = %preview,
                "dual-write SessionEvent appended",
            );
        }
        Err(err) => {
            warn!(
                %session_id,
                event = %preview,
                error = %err,
                "dual-write SessionEvent failed",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_helpers::state_with_default_model;
    use crate::domain::sessions::{Session, SessionMode};
    use crate::domain::task_status::Status as DomainStatus;
    use crate::domain::users::Username;
    use crate::routes::sessions::mount_spec_from_create_request;
    use crate::store::ReadOnlyStore;
    use chrono::Utc;
    use std::collections::HashMap;

    fn interactive_session(conversation_id: &ConversationId, status: DomainStatus) -> Session {
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            None,
            None,
            None,
            None,
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

    fn user_msg(content: &str) -> ApiSessionEvent {
        ApiSessionEvent::UserMessage {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn send_event_to_conversation_queues_when_no_entry() {
        // No worker has connected yet, so the first send must insert a
        // PendingConnection holding the event. `active_session_id` returns
        // None until set_active flips it.
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        map.send_event_to_conversation(
            &conversation_id,
            user_msg("first"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        assert!(map.active_session_id(&conversation_id).is_none());
    }

    #[tokio::test]
    async fn send_event_to_conversation_appends_to_pending() {
        // Two sends prior to any worker connecting should both queue;
        // the drain at set_active time must yield them in FIFO order.
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        for content in ["first", "second", "third"] {
            map.send_event_to_conversation(
                &conversation_id,
                user_msg(content),
                &state.store,
                ActorRef::test(),
            )
            .await
            .unwrap();
        }

        let (tx, _rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let drained = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;

        let drained_contents: Vec<String> = drained
            .into_iter()
            .map(|e| match e {
                ApiSessionEvent::UserMessage { content, .. } => content,
                other => panic!("unexpected drained variant: {other:?}"),
            })
            .collect();
        assert_eq!(drained_contents, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn send_event_to_conversation_delivers_to_active() {
        // After set_active flips the entry, a subsequent send must
        // deliver on the worker channel directly AND dual-write to the
        // session log.
        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let drained = map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;
        assert!(drained.is_empty(), "no events queued before set_active");

        let event = user_msg("hello");
        map.send_event_to_conversation(
            &conversation_id,
            event.clone(),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        let received = rx.recv().await.expect("worker channel must receive event");
        assert_eq!(received, event);

        // Dual-write hit the session log.
        let events = state.store.get_session_events(&session_id).await.unwrap();
        let hit = events.iter().any(|v| {
            matches!(
                &v.item,
                DomainSessionEvent::UserMessage { content, .. } if content == "hello"
            )
        });
        assert!(
            hit,
            "send_event_to_conversation must dual-write to the session log on Active delivery"
        );
    }

    #[tokio::test]
    async fn set_active_dual_writes_drained_pending_in_fifo() {
        // Queue three events while Pending, then set_active and confirm
        // they reach the session log in the order they were queued.
        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        for content in ["a", "b", "c"] {
            map.send_event_to_conversation(
                &conversation_id,
                user_msg(content),
                &state.store,
                ActorRef::test(),
            )
            .await
            .unwrap();
        }

        let (tx, _rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let _drained = map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;

        let events = state.store.get_session_events(&session_id).await.unwrap();
        let contents: Vec<&str> = events
            .iter()
            .filter_map(|v| match &v.item {
                DomainSessionEvent::UserMessage { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(contents, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn disconnect_removes_entry() {
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        let (tx, _rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let _ = map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;
        assert_eq!(map.active_session_id(&conversation_id), Some(session_id));

        map.disconnect(&conversation_id);
        assert!(map.active_session_id(&conversation_id).is_none());
    }

    #[tokio::test]
    async fn active_session_id_none_for_pending_entry() {
        // A PendingConnection — created implicitly by a send before any
        // worker connects — must NOT surface a session id via
        // `active_session_id`; the kill-job path treats Some only as
        // "a worker is connected to this session right now".
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        map.send_event_to_conversation(
            &conversation_id,
            user_msg("queued"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        assert!(map.active_session_id(&conversation_id).is_none());
    }
}
