use dashmap::DashMap;
use hydra_common::api::v1::relay::ServerMessage;
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
        to_worker: mpsc::Sender<ServerMessage>,
        /// `Some(agent_prompt)` iff a `Ready` has arrived but no first
        /// `UserMessage` was queued at that moment. When one arrives, the
        /// relay drains this into `ServerMessage::FirstMessage { agent_prompt,
        /// user_message }` and suppresses the redundant `Event` push for
        /// that one message.
        pending_first_message: Option<String>,
        /// Tracks whether the worker has finished its Phase-1 negotiation
        /// from the server's perspective. While `Negotiating`, the relay
        /// dual-writes events to the session log but holds them in
        /// `buffered` (instead of forwarding via `to_worker`), so a worker
        /// strictly matching `Transcript` / `FirstMessage` after sending
        /// `RequestTranscript` / `Ready` does not see a pre-Phase-1
        /// `Event` push.
        phase: ConnectionPhase,
    },
    PendingConnection {
        pending: Vec<PendingItem>,
    },
}

/// Server-side view of "is this connection past Phase-1?". The relay route
/// promotes the entry from [`ConnectionPhase::Negotiating`] to
/// [`ConnectionPhase::Ready`] once it has sent the phase-1-completing
/// message to the worker (`FirstMessage` for fresh / resume sessions,
/// `CatchUp` for mid-session reconnects).
enum ConnectionPhase {
    /// Worker is still in Phase 1 / 2. Inbound events from the chat-relay
    /// senders are dual-written to the session log and buffered here until
    /// the route calls `mark_ready`.
    Negotiating { buffered: Vec<ServerMessage> },
    /// Worker is in Phase 3. Inbound events forward via `to_worker`
    /// immediately.
    Ready,
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
    /// If `pending_first_message` is set on the active entry and the
    /// incoming event is a `UserMessage`, the relay emits
    /// `ServerMessage::FirstMessage { agent_prompt, user_message }` instead
    /// of the usual `Event` push for that single message, clears the
    /// pending flag, and suppresses the redundant `Event` push. The
    /// dual-write to the session log happens unconditionally.
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
                /// `Some(agent_prompt)` iff this UserMessage should be folded
                /// into a `FirstMessage`. `None` for any other event, or when
                /// no first-message is pending.
                pending_first_prompt: Option<String>,
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
                    pending_first_message,
                    ..
                } => {
                    let prompt = if matches!(event, ApiSessionEvent::UserMessage { .. })
                        && pending_first_message.is_some()
                    {
                        pending_first_message.take()
                    } else {
                        None
                    };
                    Decision::Active {
                        session_id: session_id.clone(),
                        pending_first_prompt: prompt,
                    }
                }
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
                pending_first_prompt,
            } => {
                let event_index = match dual_write_session_event(
                    store,
                    &session_id,
                    event.clone(),
                    actor,
                )
                .await
                {
                    Some(idx) => idx,
                    None => {
                        // The dual-write failed: the session-event log is
                        // the source of truth for `event_index`, so we
                        // cannot forward a fabricated index to the worker
                        // (it would corrupt the worker's running max for
                        // a future `Reconnecting`). Skip the forward and
                        // continue; the worker stays connected and a
                        // later send may succeed.
                        warn!(
                            %conversation_id,
                            %session_id,
                            "skipping worker forward — dual-write returned no event_index",
                        );
                        return Ok(());
                    }
                };
                let outbound = match (pending_first_prompt, event) {
                    (Some(agent_prompt), ApiSessionEvent::UserMessage { content, .. }) => {
                        ServerMessage::FirstMessage {
                            agent_prompt,
                            user_message: content,
                        }
                    }
                    (_, event) => ServerMessage::Event { event, event_index },
                };

                // Re-acquire the entry to decide forward vs buffer. The
                // phase may have transitioned between dual-write and now;
                // that's tolerated.
                enum Forward {
                    Send(ServerMessage),
                    SendAndFlush {
                        first: ServerMessage,
                        rest: Vec<ServerMessage>,
                    },
                    None,
                }
                let (to_worker_opt, forward) = {
                    match self.inner.get_mut(conversation_id) {
                        Some(mut entry) => match entry.value_mut() {
                            Entry::ActiveConnection {
                                to_worker, phase, ..
                            } => match phase {
                                ConnectionPhase::Negotiating { buffered } => {
                                    if matches!(outbound, ServerMessage::FirstMessage { .. }) {
                                        // Folding produced a FirstMessage:
                                        // phase-1 completes here. Drain the
                                        // buffer and transition to Ready so
                                        // the FirstMessage is followed by
                                        // any queued Events in order.
                                        let rest: Vec<ServerMessage> = std::mem::take(buffered);
                                        *phase = ConnectionPhase::Ready;
                                        (
                                            Some(to_worker.clone()),
                                            Forward::SendAndFlush {
                                                first: outbound,
                                                rest,
                                            },
                                        )
                                    } else {
                                        buffered.push(outbound);
                                        (None, Forward::None)
                                    }
                                }
                                ConnectionPhase::Ready => {
                                    (Some(to_worker.clone()), Forward::Send(outbound))
                                }
                            },
                            Entry::PendingConnection { .. } => (None, Forward::None),
                        },
                        None => (None, Forward::None),
                    }
                };

                match (forward, to_worker_opt) {
                    (Forward::None, _) => Ok(()),
                    (Forward::Send(msg), Some(to_worker)) => to_worker
                        .send(msg)
                        .await
                        .map_err(|_| SendEventError::ChannelClosed),
                    (Forward::SendAndFlush { first, rest }, Some(to_worker)) => {
                        to_worker
                            .send(first)
                            .await
                            .map_err(|_| SendEventError::ChannelClosed)?;
                        for m in rest {
                            to_worker
                                .send(m)
                                .await
                                .map_err(|_| SendEventError::ChannelClosed)?;
                        }
                        Ok(())
                    }
                    (Forward::Send(_) | Forward::SendAndFlush { .. }, None) => Ok(()),
                }
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
        to_worker: mpsc::Sender<ServerMessage>,
        store: &StoreWithEvents,
    ) -> Vec<(ApiSessionEvent, usize)> {
        let drained: Vec<PendingItem> = {
            let mut entry = self
                .inner
                .entry(conversation_id.clone())
                .or_insert_with(|| Entry::ActiveConnection {
                    session_id: session_id.clone(),
                    to_worker: to_worker.clone(),
                    pending_first_message: None,
                    phase: ConnectionPhase::Negotiating {
                        buffered: Vec::new(),
                    },
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
                    pending_first_message: None,
                    phase: ConnectionPhase::Negotiating {
                        buffered: Vec::new(),
                    },
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
        // preserved because the drained items are prepended into the
        // entry's `Negotiating { buffered }` buffer below before
        // `mark_ready` flushes. A later `Reconnecting` catch-up could
        // replay the log out of strict FIFO with the pending vec — the
        // race window is tiny (same user driving send_message and the
        // worker socket), and strict cross-source FIFO is left to
        // future work.
        let mut drained_events = Vec::with_capacity(drained.len());
        for item in drained {
            match dual_write_session_event(store, &session_id, item.event.clone(), item.actor).await
            {
                Some(event_index) => drained_events.push((item.event, event_index)),
                None => {
                    warn!(
                        %session_id,
                        "skipping drained pending event — dual-write returned no event_index",
                    );
                }
            }
        }

        // Install the drained events into the Negotiating buffer so that
        // they're held until the relay route calls `mark_ready` (after
        // sending the phase-1-completing `FirstMessage` to the worker).
        // Any events that arrived via concurrent
        // `send_event_to_conversation` calls between the swap above and
        // here are already at the tail of `buffered`; prepend the drained
        // items to preserve "queued before connect" → "live" FIFO from
        // the worker's perspective.
        if !drained_events.is_empty() {
            if let Some(mut entry) = self.inner.get_mut(&conversation_id) {
                if let Entry::ActiveConnection {
                    phase: ConnectionPhase::Negotiating { buffered },
                    ..
                } = entry.value_mut()
                {
                    let prepend: Vec<ServerMessage> = drained_events
                        .iter()
                        .cloned()
                        .map(|(event, event_index)| ServerMessage::Event { event, event_index })
                        .collect();
                    let mut combined = prepend;
                    combined.append(buffered);
                    *buffered = combined;
                }
                // If the phase has already transitioned to `Ready` (e.g.
                // a concurrent fold flushed the buffer), the drained
                // events are kept only on the returned vec — the caller
                // is responsible for forwarding them in that uncommon
                // race. Today no caller exercises that path.
            }
        }

        drained_events
    }

    /// Transition the entry's [`ConnectionPhase`] from `Negotiating` to
    /// `Ready` and return the events that were buffered during
    /// negotiation. The relay route calls this after sending the
    /// phase-1-completing message to the worker (`FirstMessage` for
    /// fresh / resume sessions, `CatchUp` for reconnects) and forwards
    /// the returned messages on the worker WebSocket in order.
    ///
    /// `dedupe_user_message` should be `Some(content)` iff the route
    /// already delivered a `UserMessage` with that exact content to the
    /// worker as part of the phase-1-completing message (e.g. folded
    /// into `FirstMessage.user_message`). The first buffered `Event`
    /// `UserMessage` with matching content is then removed from the
    /// returned buffer to avoid duplicate delivery. If the buffer holds
    /// no matching event (e.g. the fold drew from a prior session's
    /// log), nothing is removed.
    ///
    /// Returns an empty vec if the entry is already `Ready`, or if no
    /// entry exists.
    pub fn mark_ready(
        &self,
        conversation_id: &ConversationId,
        dedupe_user_message: Option<&str>,
    ) -> Vec<ServerMessage> {
        let mut entry = match self.inner.get_mut(conversation_id) {
            Some(e) => e,
            None => return Vec::new(),
        };
        match entry.value_mut() {
            Entry::ActiveConnection { phase, .. } => match phase {
                ConnectionPhase::Negotiating { buffered } => {
                    let mut drained = std::mem::take(buffered);
                    if let Some(target) = dedupe_user_message {
                        if let Some(pos) = drained.iter().position(|m| {
                            matches!(
                                m,
                                ServerMessage::Event {
                                    event: ApiSessionEvent::UserMessage { content, .. },
                                    ..
                                } if content == target
                            )
                        }) {
                            drained.remove(pos);
                        }
                    }
                    *phase = ConnectionPhase::Ready;
                    drained
                }
                ConnectionPhase::Ready => Vec::new(),
            },
            Entry::PendingConnection { .. } => {
                warn!(%conversation_id, "mark_ready called on PendingConnection; ignoring");
                Vec::new()
            }
        }
    }

    /// Stash a pending `agent_prompt` on the conversation's active entry
    /// so the next inbound `UserMessage` is rewritten into a
    /// `ServerMessage::FirstMessage` (§1.5 of the design). Returns
    /// `false` if the entry isn't `ActiveConnection` (shouldn't happen —
    /// `Ready` arrives after `set_active`).
    pub fn set_pending_first_message(
        &self,
        conversation_id: &ConversationId,
        agent_prompt: String,
    ) -> bool {
        match self.inner.get_mut(conversation_id) {
            Some(mut entry) => match entry.value_mut() {
                Entry::ActiveConnection {
                    pending_first_message,
                    ..
                } => {
                    *pending_first_message = Some(agent_prompt);
                    true
                }
                Entry::PendingConnection { .. } => {
                    warn!(
                        %conversation_id,
                        "set_pending_first_message called on PendingConnection entry; ignoring"
                    );
                    false
                }
            },
            None => {
                warn!(
                    %conversation_id,
                    "set_pending_first_message called with no entry; ignoring"
                );
                false
            }
        }
    }

    #[cfg(test)]
    fn phase_is_ready(&self, conversation_id: &ConversationId) -> Option<bool> {
        self.inner
            .get(conversation_id)
            .and_then(|entry| match entry.value() {
                Entry::ActiveConnection { phase, .. } => {
                    Some(matches!(phase, ConnectionPhase::Ready))
                }
                Entry::PendingConnection { .. } => None,
            })
    }

    /// Remove the entry entirely. Called when a worker WebSocket
    /// disconnects so a subsequent reconnect starts from a clean slate.
    pub fn disconnect(&self, conversation_id: &ConversationId) {
        self.inner.remove(conversation_id);
    }

    /// Push `ServerMessage::EndSession` onto the conversation's connected
    /// worker via its `to_worker` channel. Returns `true` if a connected
    /// worker received the message (channel was open). Returns `false`
    /// when there is no entry, the entry is still `PendingConnection`, or
    /// the worker's mpsc channel is closed/full.
    ///
    /// This is a protocol message (not a `SessionEvent`) — no dual-write
    /// to the session log. The graceful close in `close_conversation`
    /// uses the existing WS-close `cleanup` path as the implicit ack:
    /// when the worker finishes the unified end-of-session sequence and
    /// closes the WS, `pump_phase3 → cleanup` drops the relay entry, so
    /// `active_session_id` flips to `None`.
    pub fn send_end_session(&self, conversation_id: &ConversationId) -> bool {
        let to_worker = match self.inner.get(conversation_id) {
            Some(entry) => match entry.value() {
                Entry::ActiveConnection { to_worker, .. } => to_worker.clone(),
                Entry::PendingConnection { .. } => return false,
            },
            None => return false,
        };
        match to_worker.try_send(ServerMessage::EndSession) {
            Ok(()) => true,
            Err(err) => {
                warn!(
                    %conversation_id,
                    error = %err,
                    "send_end_session: to_worker channel send failed"
                );
                false
            }
        }
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

/// Dual-write an event to the session's event log. Returns the per-session
/// `VersionNumber` assigned by the store (as a `usize`, used by callers as
/// the `event_index` they ship to the worker over the relay). Returns
/// `None` on a try_into / store failure: callers must skip the worker
/// forward in that case, since the worker tracks indices for the
/// reconnect-and-resume protocol and a fabricated index would corrupt its
/// running max.
async fn dual_write_session_event(
    store: &StoreWithEvents,
    session_id: &SessionId,
    event: ApiSessionEvent,
    actor: ActorRef,
) -> Option<usize> {
    let domain_event: SessionEvent = match event.try_into() {
        Ok(e) => e,
        Err(_) => {
            warn!(
                %session_id,
                "dual-write skipped: unknown SessionEvent variant",
            );
            return None;
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
            Some(version as usize)
        }
        Err(err) => {
            warn!(
                %session_id,
                event = %preview,
                error = %err,
                "dual-write SessionEvent failed",
            );
            None
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
    use crate::store::ReadOnlyStore;
    use chrono::Utc;
    use std::collections::HashMap;

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
                greet_user: false,
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

    fn assistant_msg(content: &str) -> ApiSessionEvent {
        ApiSessionEvent::AssistantMessage {
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
            .iter()
            .map(|(e, _)| match e {
                ApiSessionEvent::UserMessage { content, .. } => content.clone(),
                other => panic!("unexpected drained variant: {other:?}"),
            })
            .collect();
        assert_eq!(drained_contents, vec!["first", "second", "third"]);

        // Each drained item is paired with the per-session event_index
        // assigned by the dual-write — versions start at 1 and increase
        // monotonically, so this slice is `[1, 2, 3]`.
        let indices: Vec<usize> = drained.iter().map(|(_, idx)| *idx).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn send_event_to_conversation_delivers_to_active() {
        // After set_active flips the entry and mark_ready promotes the
        // phase out of Negotiating, a subsequent send must deliver on
        // the worker channel directly AND dual-write to the session log.
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
        let flushed = map.mark_ready(&conversation_id, None);
        assert!(flushed.is_empty(), "nothing buffered for an empty drain");

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
        assert_eq!(
            received,
            ServerMessage::Event {
                event,
                event_index: 1,
            }
        );

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

    #[tokio::test]
    async fn pending_first_message_folds_into_first_message() {
        // After set_active, set a pending first-message prompt. The
        // next inbound UserMessage must be forwarded as FirstMessage
        // (not Event) and the dual-write to the session log still happens.
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
        let _ = map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;
        // The fresh path stashes pending_first_message before mark_ready,
        // so leave the entry in Negotiating here — the fold + flush path
        // is exercised by `negotiating_fold_flushes_buffer` below.
        let _ = map.mark_ready(&conversation_id, None);

        let stashed = map.set_pending_first_message(&conversation_id, "be helpful".to_string());
        assert!(stashed, "set_pending_first_message must succeed on Active");

        map.send_event_to_conversation(
            &conversation_id,
            user_msg("hello"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        let received = rx.recv().await.expect("worker channel must receive");
        assert_eq!(
            received,
            ServerMessage::FirstMessage {
                agent_prompt: "be helpful".to_string(),
                user_message: "hello".to_string(),
            }
        );

        // A subsequent UserMessage is forwarded as a regular Event
        // since the pending flag was cleared.
        map.send_event_to_conversation(
            &conversation_id,
            user_msg("again"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();
        let received = rx.recv().await.unwrap();
        let (timestamp, event_index) = match &received {
            ServerMessage::Event {
                event: ApiSessionEvent::UserMessage { timestamp, .. },
                event_index,
            } => (*timestamp, *event_index),
            other => panic!("expected Event UserMessage, got {other:?}"),
        };
        assert_eq!(
            received,
            ServerMessage::Event {
                event: ApiSessionEvent::UserMessage {
                    content: "again".to_string(),
                    timestamp,
                },
                event_index,
            }
        );
        assert!(event_index >= 1, "event_index must be assigned");
    }

    #[tokio::test]
    async fn pending_first_message_does_not_fold_non_user_message() {
        // An AssistantMessage arriving while pending_first_message is
        // set should NOT consume the pending prompt.
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
        let _ = map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;
        let _ = map.mark_ready(&conversation_id, None);

        map.set_pending_first_message(&conversation_id, "still-pending".to_string());

        let assistant = assistant_msg("model talking");
        map.send_event_to_conversation(
            &conversation_id,
            assistant.clone(),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ServerMessage::Event {
                event: ref got,
                event_index,
            } => {
                assert_eq!(got, &assistant);
                assert!(event_index >= 1, "event_index must be assigned");
            }
            other => panic!("expected Event AssistantMessage, got {other:?}"),
        }

        // Pending prompt still set: a later UserMessage folds it.
        map.send_event_to_conversation(
            &conversation_id,
            user_msg("now"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();
        let received = rx.recv().await.unwrap();
        assert!(matches!(received, ServerMessage::FirstMessage { .. }));
    }

    #[tokio::test]
    async fn set_active_starts_in_negotiating_phase() {
        // After set_active, the phase is Negotiating: the route hasn't
        // yet sent FirstMessage/CatchUp, so inbound events must NOT
        // forward to to_worker until mark_ready is called.
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
        let _ = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;
        assert_eq!(map.phase_is_ready(&conversation_id), Some(false));

        // Sending an event right now must buffer, not forward.
        map.send_event_to_conversation(
            &conversation_id,
            user_msg("buffered"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "Negotiating phase must NOT forward events to to_worker"
        );

        // mark_ready promotes to Ready and returns the buffered events.
        let flushed = map.mark_ready(&conversation_id, None);
        assert_eq!(map.phase_is_ready(&conversation_id), Some(true));
        assert_eq!(flushed.len(), 1);
        assert!(matches!(
            &flushed[0],
            ServerMessage::Event {
                event: ApiSessionEvent::UserMessage { content, .. }, ..
            } if content == "buffered"
        ));
    }

    #[tokio::test]
    async fn set_active_buffers_drained_pending_for_replay() {
        // When set_active drains a non-empty PendingConnection, the
        // drained Events should land in the Negotiating buffer (in FIFO
        // order) — NOT immediately forwarded to to_worker — so the
        // worker doesn't see Event pushes before its Phase-1 reply has
        // arrived.
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        // Queue some events while no worker is connected.
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

        let (tx, mut rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let _ = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;

        assert!(
            rx.try_recv().is_err(),
            "set_active must buffer drained events, not forward them pre-Phase-1"
        );

        // mark_ready returns them in order for the route to ship over
        // the WS after FirstMessage/CatchUp.
        let flushed = map.mark_ready(&conversation_id, None);
        let contents: Vec<&str> = flushed
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Event {
                    event: ApiSessionEvent::UserMessage { content, .. },
                    ..
                } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(contents, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn mark_ready_dedupes_first_user_message_when_folded() {
        // When the relay route folds the first drained UserMessage into
        // FirstMessage.user_message, mark_ready must NOT replay that
        // same UserMessage as an Event — otherwise the worker would
        // receive the content twice.
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        for content in ["msg1", "msg2"] {
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
        let _ = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;

        // Pretend the route folded msg1 into FirstMessage.
        let flushed = map.mark_ready(&conversation_id, Some("msg1"));
        let contents: Vec<&str> = flushed
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Event {
                    event: ApiSessionEvent::UserMessage { content, .. },
                    ..
                } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            contents,
            vec!["msg2"],
            "matching UserMessage must be removed from the flushed buffer"
        );
    }

    #[tokio::test]
    async fn mark_ready_does_not_dedupe_non_matching_content() {
        // When the route folds a UserMessage that lives in a *prior*
        // session's log (e.g. on resume the conversation's original
        // first message comes from the old session), the current
        // session's buffer does NOT hold an Event with that content.
        // `mark_ready` must then leave all buffered Events intact.
        let state = state_with_default_model("default-model");
        let map = ChatRelayMap::new();
        let conversation_id = ConversationId::new();

        let session = interactive_session(&conversation_id, DomainStatus::Running);
        let (session_id, _) = state
            .store
            .add_session_with_actor(session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        map.send_event_to_conversation(
            &conversation_id,
            user_msg("new-turn"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();
        let (tx, _rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let _ = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;

        // Pretend the route folded a different UserMessage from a
        // prior-session log into FirstMessage.
        let flushed = map.mark_ready(&conversation_id, Some("prior-session-message"));
        let contents: Vec<&str> = flushed
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Event {
                    event: ApiSessionEvent::UserMessage { content, .. },
                    ..
                } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            contents,
            vec!["new-turn"],
            "non-matching dedup content must NOT remove any buffered events"
        );
    }

    #[tokio::test]
    async fn negotiating_fold_flushes_buffer_after_first_message() {
        // The fresh-path stash case: handle_ready couldn't find a first
        // UserMessage, so it stashed `pending_first_message` and the
        // entry stayed in Negotiating. When the user POST eventually
        // arrives, send_event_to_conversation folds it into
        // FirstMessage, transitions to Ready, and flushes any buffered
        // events afterwards — in order, FirstMessage first.
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
        let _ = map
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;
        // Stash, mimicking handle_ready's "no first user message yet" path.
        assert!(map.set_pending_first_message(&conversation_id, "prompt".to_string()));

        // An AssistantMessage racing in DURING Negotiating goes to the
        // buffer (doesn't consume pending_first_message).
        map.send_event_to_conversation(
            &conversation_id,
            assistant_msg("tail-end"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "AssistantMessage must remain buffered while Negotiating"
        );

        // The user POST arrives — folds + transitions + flushes.
        map.send_event_to_conversation(
            &conversation_id,
            user_msg("real-first"),
            &state.store,
            ActorRef::test(),
        )
        .await
        .unwrap();

        let first = rx.recv().await.expect("FirstMessage must arrive");
        assert_eq!(
            first,
            ServerMessage::FirstMessage {
                agent_prompt: "prompt".to_string(),
                user_message: "real-first".to_string(),
            }
        );
        let second = rx.recv().await.expect("buffered Assistant must follow");
        assert!(matches!(
            second,
            ServerMessage::Event {
                event: ApiSessionEvent::AssistantMessage { .. },
                ..
            }
        ));
        assert_eq!(map.phase_is_ready(&conversation_id), Some(true));
    }

    #[tokio::test]
    async fn mark_ready_is_noop_on_ready_phase() {
        // mark_ready called twice is safe: the second call returns an
        // empty vec and leaves the phase Ready.
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
            .set_active(conversation_id.clone(), session_id, tx, &state.store)
            .await;
        let _ = map.mark_ready(&conversation_id, None);
        let second = map.mark_ready(&conversation_id, None);
        assert!(second.is_empty());
        assert_eq!(map.phase_is_ready(&conversation_id), Some(true));
    }
}
