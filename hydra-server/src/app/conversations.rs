use crate::{
    app::AgentError,
    domain::{
        actors::ActorRef,
        conversations::{Conversation, ConversationEvent, ConversationStatus},
        sessions::SessionEvent,
        users::Username,
    },
    store::StoreError,
};
use hydra_common::{
    ConversationId, Versioned,
    api::v1::{agents::AgentName, sessions as api_sessions, sessions::SearchSessionsQuery},
};
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};

use super::app_state::AppState;

/// Bounded deadline for the graceful End Chat path in
/// [`AppState::close_conversation`]. After sending `ServerMessage::EndSession`
/// the server waits up to this long for the worker to upload its session
/// state, ack, and close the WS (observed as the relay entry going away).
/// On timeout it falls back to `job_engine.kill_job` so a stuck worker
/// cannot block End Chat.
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// Poll cadence for the active-session-id deadline poll. The shortest
/// observable end-to-end shutdown today is bounded by the worker's final
/// `SessionStateUpload` + `Closed` + `EndSessionAck` round-trip, so a
/// 100ms cadence keeps wall-clock overhead negligible without busy-looping.
const GRACEFUL_CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum CreateConversationError {
    #[error("failed to store conversation")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("agent '{name}' not found")]
    AgentNotFound { name: String },
    #[error("failed to resolve agent")]
    Agent {
        #[source]
        source: AgentError,
    },
}

#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("failed to access conversation store")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("principal '{principal}' is not the conversation creator")]
    Forbidden { principal: Username },
}

#[derive(Debug, Error)]
pub enum CloseConversationError {
    #[error("failed to access conversation store")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum ResumeConversationError {
    #[error("failed to access conversation store")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("conversation is already active")]
    AlreadyActive,
}

impl AppState {
    pub async fn create_conversation(
        &self,
        message: Option<String>,
        agent_name: Option<AgentName>,
        session_settings: crate::domain::issues::SessionSettings,
        actor_ref: ActorRef,
        creator: Username,
    ) -> Result<(ConversationId, Versioned<Conversation>), CreateConversationError> {
        // Validate explicit `agent_name` synchronously so a typo on the
        // client side surfaces as a 4xx instead of a silently-spawnless
        // 200. When `agent_name` is `None` we deliberately skip this check —
        // the absence of a registered default conversation agent is a
        // server-config concern handled by the automation, not a
        // client-input error.
        if let Some(name) = agent_name.as_ref() {
            match self.resolve_conversation_agent(Some(name.as_str())).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(CreateConversationError::AgentNotFound {
                        name: name.as_str().to_string(),
                    });
                }
                Err(AgentError::NotFound { name }) => {
                    return Err(CreateConversationError::AgentNotFound { name });
                }
                Err(source) => {
                    return Err(CreateConversationError::Agent { source });
                }
            }
        }

        // Persist the conversation in Active status. The companion session
        // is spawned asynchronously by `SpawnConversationSessionsAutomation`
        // (in `policy/automations/spawn_conversation_sessions.rs`) when the
        // ConversationCreated event lands on the bus.
        let conversation = Conversation {
            title: None,
            agent_name,
            status: ConversationStatus::Active,
            creator: creator.clone(),
            session_settings,
            deleted: false,
        };

        let (conversation_id, _version) = self
            .store
            .add_conversation_with_actor(conversation, actor_ref.clone())
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        // Deliver the optional first user message through `send_message` so
        // it both lands in the event log AND attempts the relay path. The
        // worker is still being spawned by
        // `SpawnConversationSessionsAutomation` at this point, so the relay
        // call will normally log "no relay connected, worker will catch up"
        // and the message will be picked up via catch-up. But if the worker
        // wins the race and connects before this call, the relay path
        // delivers the message directly — and `PromptPrepend`'s
        // first-`UserMessage` branch in the relay loop consumes the agent
        // prompt prepend from there.
        if let Some(content) = message {
            self.send_message(&conversation_id, content, actor_ref, creator)
                .await
                .map_err(|err| match err {
                    SendMessageError::Store { source } => CreateConversationError::Store { source },
                    // Unreachable: we just created the conversation with
                    // `creator`, so the creator-match check inside
                    // `send_message` will pass on this immediate follow-up
                    // call.
                    SendMessageError::Forbidden { principal } => {
                        CreateConversationError::Store {
                            source: StoreError::Internal(format!(
                                "unexpected forbidden during create_conversation follow-up send_message for principal '{principal}'",
                            )),
                        }
                    }
                })?;
        }

        let versioned = self
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        info!(conversation_id = %conversation_id, "conversation created");
        Ok((conversation_id, versioned))
    }

    pub async fn send_message(
        &self,
        conversation_id: &ConversationId,
        content: String,
        actor_ref: ActorRef,
        principal: Username,
    ) -> Result<api_sessions::SessionEvent, SendMessageError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| SendMessageError::Store { source })?;

        // Creator-only gate: a conversation may only be appended to by the
        // user that created it. The check lives here (rather than in the
        // route handler) so any future internal caller of `send_message` is
        // also covered.
        if versioned.item.creator != principal {
            return Err(SendMessageError::Forbidden { principal });
        }

        // If not Active, transparently flip to Active before recording the
        // new message. The companion session — and the corresponding Resumed
        // event — are produced asynchronously by
        // `SpawnConversationSessionsAutomation` when the ConversationUpdated
        // event lands on the bus.
        if versioned.item.status != ConversationStatus::Active {
            let mut updated = versioned.item;
            updated.status = ConversationStatus::Active;
            self.store
                .update_conversation_with_actor(conversation_id, updated, actor_ref.clone())
                .await
                .map_err(|source| SendMessageError::Store { source })?;
        }

        // Hand the UserMessage off to the chat-relay layer. When a worker
        // is connected, the relay both dual-writes to the session event
        // log AND forwards over the per-conversation channel. When no
        // worker is connected yet (a brand-new or just-reactivated
        // conversation whose companion session is still being spawned by
        // `SpawnConversationSessionsAutomation`), the event is queued
        // and delivered atomically when the worker connects — preserving
        // the Phase E invariant that UserMessage lives on the session
        // log without forcing this path to block on a session lookup.
        let event = SessionEvent::UserMessage {
            content,
            timestamp: chrono::Utc::now(),
        };
        let api_event: api_sessions::SessionEvent = event.into();
        match self
            .chat_relay_map
            .send_event_to_conversation(conversation_id, api_event.clone(), &self.store, actor_ref)
            .await
        {
            Ok(()) => {
                info!(conversation_id = %conversation_id, "send_message accepted");
            }
            Err(err) => {
                warn!(conversation_id = %conversation_id, error = %err, "send_message: relay forward failed");
            }
        }

        Ok(api_event)
    }

    pub async fn close_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_ref: ActorRef,
    ) -> Result<Versioned<Conversation>, CloseConversationError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        // Idempotent: if already Closed, return as-is
        if versioned.item.status == ConversationStatus::Closed {
            return Ok(versioned);
        }

        // Append Closed event
        let event = ConversationEvent::Closed {
            timestamp: chrono::Utc::now(),
        };
        self.store
            .append_conversation_event_with_actor(conversation_id, event.clone(), actor_ref.clone())
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        // Dual-write the equivalent SessionEvent onto the conversation's
        // active session (Phase C step 7 of the sessions-orthogonality
        // redesign). At this point the worker is still alive (we kill it
        // below), so the active relay entry — and the session it points to
        // — is the right target.
        //
        // The match keeps a full arm for each lifecycle variant even though
        // close emits `Closed` in practice — other lifecycle write paths
        // may grow into this surface later. `Resumed` is `None` here
        // because the producing session id is the new session and the
        // prior `from_session_id` only exists in the spawn automation,
        // which writes that `SessionEvent::Resumed` directly.
        let session_event = match &event {
            ConversationEvent::Suspending { reason, timestamp } => Some(SessionEvent::Suspending {
                reason: reason.clone(),
                timestamp: *timestamp,
            }),
            ConversationEvent::Closed { timestamp } => Some(SessionEvent::Closed {
                timestamp: *timestamp,
            }),
            ConversationEvent::Resumed { .. } => None,
        };
        if let Some(session_event) = session_event {
            // Resolve the session backing this conversation without an
            // active-state filter: a lifecycle `Closed` may land after the
            // worker session has already gone terminal (e.g. the worker
            // exited before the user clicked close), and we still want the
            // event on its log. Prefer the actively-connected session
            // (which by definition is the one currently relaying); fall
            // back to the most-recent session of any status.
            let resolved_session_id = match self.chat_relay_map.active_session_id(conversation_id) {
                Some(id) => Some(id),
                None => {
                    let mut query = SearchSessionsQuery::default();
                    query.conversation_id = Some(conversation_id.clone());
                    self.store()
                        .list_sessions(&query)
                        .await
                        .ok()
                        .and_then(|sessions| {
                            sessions
                                .into_iter()
                                .max_by_key(|(_, v)| v.creation_time)
                                .map(|(id, _)| id)
                        })
                }
            };
            if let Some(session_id) = resolved_session_id {
                let preview = session_event.preview();
                match self
                    .store
                    .append_session_event_with_actor(&session_id, session_event, actor_ref.clone())
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
            } else {
                warn!(
                    %conversation_id,
                    "dual-write SessionEvent skipped: no session linked to conversation yet"
                );
            }
        }

        // Drive the active worker (if any) through graceful shutdown.
        // Send `ServerMessage::EndSession` over the relay and await the
        // worker's clean WS close (which `relay.rs::cleanup` mirrors as
        // the relay entry being dropped). If the worker doesn't disconnect
        // within `GRACEFUL_CLOSE_TIMEOUT`, fall back to `kill_job` — the
        // pre-graceful behavior — so stuck workers can't block End Chat.
        if let Some(session_id) = self.chat_relay_map.active_session_id(conversation_id) {
            let sent = self.chat_relay_map.send_end_session(conversation_id);
            if sent {
                info!(
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    "sent EndSession; awaiting worker WS close"
                );
            } else {
                warn!(
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    "send_end_session found no live to_worker channel; will fall back to kill_job"
                );
            }
            let exited_cleanly = sent && {
                let conv_id = conversation_id.clone();
                let chat_relay_map = self.chat_relay_map.clone();
                let poll_fut = async move {
                    loop {
                        if chat_relay_map.active_session_id(&conv_id).is_none() {
                            return true;
                        }
                        tokio::time::sleep(GRACEFUL_CLOSE_POLL_INTERVAL).await;
                    }
                };
                tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, poll_fut)
                    .await
                    .unwrap_or(false)
            };

            if exited_cleanly {
                info!(
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    "worker exited cleanly after EndSession"
                );
                // Revoke tokens minted by this session so any inflight
                // request from the dying container fails at `require_auth`.
                // `kill_session` does this in its own success branch; the
                // natural-exit path has no automation that does the
                // equivalent, so we explicitly do it here to keep parity
                // with the `kill_job` fallback below.
                if let Err(err) = self.store.revoke_auth_tokens_for_session(&session_id).await {
                    warn!(
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        error = %err,
                        "failed to revoke session tokens after graceful close"
                    );
                }
            } else {
                warn!(
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    "worker did not exit within {GRACEFUL_CLOSE_TIMEOUT:?} after EndSession; falling back to kill_job"
                );
                match self.job_engine.kill_job(&session_id).await {
                    Ok(()) => {
                        info!(conversation_id = %conversation_id, session_id = %session_id, "killed active session");
                    }
                    Err(err) => {
                        warn!(conversation_id = %conversation_id, session_id = %session_id, error = %err, "failed to kill session (may already be stopped)");
                    }
                }
                if let Err(err) = self.store.revoke_auth_tokens_for_session(&session_id).await {
                    warn!(
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        error = %err,
                        "failed to revoke session tokens after kill_job fallback"
                    );
                }
            }
        }

        // Update conversation status
        let mut updated = versioned.item;
        updated.status = ConversationStatus::Closed;
        self.store
            .update_conversation_with_actor(conversation_id, updated, actor_ref)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        // Return updated conversation
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        Ok(versioned)
    }

    pub async fn update_conversation_metadata(
        &self,
        conversation_id: &ConversationId,
        title: Option<String>,
        actor_ref: ActorRef,
    ) -> Result<Versioned<Conversation>, CloseConversationError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        let mut updated = versioned.item;
        if let Some(title) = title {
            updated.title = Some(title);
        }

        self.store
            .update_conversation_with_actor(conversation_id, updated, actor_ref)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        Ok(versioned)
    }

    pub async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_ref: ActorRef,
    ) -> Result<Versioned<Conversation>, CloseConversationError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        let mut updated = versioned.item;
        updated.deleted = true;

        self.store
            .update_conversation_with_actor(conversation_id, updated, actor_ref)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        let versioned = self
            .store()
            .get_conversation(conversation_id, true)
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        Ok(versioned)
    }

    pub async fn resume_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_ref: ActorRef,
    ) -> Result<Versioned<Conversation>, ResumeConversationError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        if versioned.item.status == ConversationStatus::Active {
            return Err(ResumeConversationError::AlreadyActive);
        }

        // Flip to Active. The session spawn, the `conversation_resume_from`
        // snapshot, and the `Resumed` event are all driven asynchronously
        // by `SpawnConversationSessionsAutomation`.
        let mut updated = versioned.item;
        updated.status = ConversationStatus::Active;
        self.store
            .update_conversation_with_actor(conversation_id, updated, actor_ref)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        Ok(versioned)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::chat_relay::TO_WORKER_CAPACITY;
    use crate::{
        app::{
            AppState,
            test_helpers::{poll_until, start_test_automation_runner, state_with_default_model},
        },
        domain::{
            actors::ActorRef,
            agents::Agent,
            conversations::{ConversationEvent, ConversationStatus},
            documents::Document,
            issues::SessionSettings,
            sessions::Session,
            users::Username,
        },
        policy::automations::agent_queue::AGENT_NAME_ENV_VAR,
    };
    use hydra_common::{
        ConversationId, SessionId, Versioned, api::v1::sessions::SearchSessionsQuery,
    };
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Simulate a worker connecting to the conversation's relay so the
    /// chat_relay layer transitions to `ActiveConnection`. Drains any
    /// queued events into the given session's log (dual-write) and
    /// returns the per-conversation worker receiver — held by the
    /// caller so subsequent dual-writes can drain it as needed. In
    /// production this is what `handle_relay_socket` does after the
    /// catch-up handshake.
    async fn simulate_worker_connect(
        state: &AppState,
        conversation_id: &ConversationId,
        session_id: &SessionId,
    ) -> mpsc::Receiver<hydra_common::api::v1::conversations::ServerMessage> {
        let (tx, rx) = mpsc::channel(TO_WORKER_CAPACITY);
        let _ = state
            .chat_relay_map
            .set_active(
                conversation_id.clone(),
                session_id.clone(),
                tx,
                &state.store,
            )
            .await;
        rx
    }

    /// How long tests will wait for the spawn-conversation-sessions automation
    /// to settle. The runner processes events from the bus on a separate
    /// task, so a brief poll loop is required after any mutation that should
    /// trigger a spawn.
    const POLL_TIMEOUT: Duration = Duration::from_secs(5);

    /// Wait for at least one session linked to the given conversation to
    /// appear in the store, then return the most-recently-created one.
    async fn wait_for_session(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> Versioned<Session> {
        poll_until(POLL_TIMEOUT, || async {
            let sessions = state
                .store()
                .list_sessions(&SearchSessionsQuery::default())
                .await
                .unwrap();
            sessions
                .into_iter()
                .filter_map(|(_, s)| {
                    (s.item.conversation_id() == Some(conversation_id)).then_some(s)
                })
                .max_by_key(|s| s.creation_time)
        })
        .await
        .expect("expected a session for the conversation to appear")
    }

    /// Wait for at least `expected_count` sessions linked to the conversation
    /// to exist.
    async fn wait_for_session_count(
        state: &AppState,
        conversation_id: &ConversationId,
        expected_count: usize,
    ) {
        let result = poll_until(POLL_TIMEOUT, || async {
            let sessions = state
                .store()
                .list_sessions(&SearchSessionsQuery::default())
                .await
                .unwrap();
            let count = sessions
                .into_iter()
                .filter(|(_, s)| s.item.conversation_id() == Some(conversation_id))
                .count();
            (count >= expected_count).then_some(count)
        })
        .await;
        assert!(
            result.is_some(),
            "expected at least {expected_count} sessions for conversation",
        );
    }

    /// Wait for a `Resumed` event to appear in the conversation's event log
    /// and return the session_id it references.
    async fn wait_for_resumed_event(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> hydra_common::SessionId {
        poll_until(POLL_TIMEOUT, || async {
            let events = state
                .store()
                .get_conversation_events(conversation_id)
                .await
                .unwrap();
            events.into_iter().rev().find_map(|e| match e.item {
                ConversationEvent::Resumed { session_id, .. } => Some(session_id),
                _ => None,
            })
        })
        .await
        .expect("expected a Resumed event to appear")
    }

    /// Register an agent and an accompanying prompt document.
    async fn register_agent_with_prompt(
        state: &AppState,
        name: &str,
        prompt_body: &str,
        is_default: bool,
        secrets: Vec<String>,
    ) {
        let prompt_path = format!("/agents/{name}/prompt.md");
        let agent = Agent::new(
            name.to_string(),
            prompt_path.clone(),
            None,
            1,
            1,
            false,
            is_default,
            secrets,
        );
        state.store.add_agent(agent).await.unwrap();

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: prompt_body.to_string(),
            path: Some(prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();
    }

    /// Register an agent with both a prompt and an MCP config document.
    async fn register_agent_with_prompt_and_mcp(
        state: &AppState,
        name: &str,
        prompt_body: &str,
        mcp_body: &str,
        secrets: Vec<String>,
    ) {
        let prompt_path = format!("/agents/{name}/prompt.md");
        let mcp_path = format!("/agents/{name}/mcp.json");
        let agent = Agent::new(
            name.to_string(),
            prompt_path.clone(),
            Some(mcp_path.clone()),
            1,
            1,
            false,
            false,
            secrets,
        );
        state.store.add_agent(agent).await.unwrap();

        let prompt_doc = Document {
            title: format!("{name} prompt"),
            body_markdown: prompt_body.to_string(),
            path: Some(prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(prompt_doc, ActorRef::test())
            .await
            .unwrap();

        let mcp_doc = Document {
            title: format!("{name} mcp config"),
            body_markdown: mcp_body.to_string(),
            path: Some(mcp_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(mcp_doc, ActorRef::test())
            .await
            .unwrap();
    }

    /// Look up the (single) session associated with a conversation, polling
    /// briefly to give the spawn-conversation-sessions automation time to
    /// create it.
    async fn session_for_conversation(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> Versioned<Session> {
        wait_for_session(state, conversation_id).await
    }

    /// Drive an existing session to a terminal status via the event-emitting
    /// `update_session_with_actor` path. Required when a test that closes a
    /// conversation needs to then send a message into the resume path: with
    /// the active-state filter on the chat_relay resolver, a prior session
    /// that is still in `Created`/`Pending`/`Running` would otherwise be
    /// returned by the resolver instead of waiting for the new spawn. In
    /// production this transition happens via the kill_job + monitor flow;
    /// in unit tests we simulate it directly.
    async fn mark_session_terminal(
        state: &AppState,
        session_id: &hydra_common::SessionId,
        status: crate::domain::task_status::Status,
    ) {
        let mut session = state
            .store()
            .get_session(session_id, false)
            .await
            .expect("session must exist")
            .item;
        session.status = status;
        state
            .store
            .update_session_with_actor(session_id, session, ActorRef::test())
            .await
            .expect("update_session_with_actor must succeed");
    }

    /// Look up the most-recent session_id for a conversation, polling
    /// briefly to give the spawn-conversation-sessions automation time to
    /// create it. Useful for fetching session-event logs in tests that
    /// previously asserted on the conversation-events log.
    async fn session_id_for_conversation(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> hydra_common::SessionId {
        poll_until(POLL_TIMEOUT, || async {
            let sessions = state
                .store()
                .list_sessions(&SearchSessionsQuery::default())
                .await
                .unwrap();
            sessions
                .into_iter()
                .filter(|(_, s)| s.item.conversation_id() == Some(conversation_id))
                .max_by_key(|(_, s)| s.creation_time)
                .map(|(id, _)| id)
        })
        .await
        .expect("expected a session_id for the conversation to appear")
    }

    #[tokio::test]
    async fn create_conversation_applies_session_settings_model() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        // A conversation needs an agent for the automation to spawn a session.
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let (conversation_id, versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        assert_eq!(versioned.item.status, ConversationStatus::Active);
        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(
            session.item.agent_config.model.as_deref(),
            Some("custom-model")
        );
    }

    #[tokio::test]
    async fn create_conversation_applies_default_model_from_config() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(
            session.item.agent_config.model.as_deref(),
            Some("default-model")
        );
    }

    #[tokio::test]
    async fn create_conversation_applies_remote_url_to_context() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings {
            remote_url: Some("https://github.com/org/repo.git".to_string()),
            branch: Some("feature".to_string()),
            ..Default::default()
        };

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        use hydra_common::api::v1::sessions::{Bundle, MountItem};
        let bundle = session
            .item
            .mount_spec
            .mounts
            .iter()
            .find_map(|m| match m {
                MountItem::Bundle { bundle, .. } => Some(bundle.clone()),
                _ => None,
            })
            .expect("mount_spec must carry a Bundle item");
        match bundle {
            Bundle::GitRepository { url, rev } => {
                assert_eq!(url, "https://github.com/org/repo.git");
                assert_eq!(rev, "feature");
            }
            other => panic!("expected GitRepository, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_conversation_applies_session_settings_secrets() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings {
            secrets: Some(vec!["GH_TOKEN".to_string()]),
            ..Default::default()
        };

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.secrets, Some(vec!["GH_TOKEN".to_string()]));
    }

    #[tokio::test]
    async fn create_conversation_sets_interactive_and_conversation_id() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert!(
            session.item.is_interactive(),
            "conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id().cloned(),
            Some(conversation_id),
            "conversation session should have conversation_id set"
        );
    }

    #[tokio::test]
    async fn create_conversation_with_no_message_starts_with_zero_events() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, versioned) = state
            .create_conversation(
                None,
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        assert_eq!(versioned.item.status, ConversationStatus::Active);

        // Wait for the session to spawn before reading conversation events,
        // otherwise we may race with the automation appending its own state.
        let session = session_for_conversation(&state, &conversation_id).await;
        assert!(
            session.item.is_interactive(),
            "conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id().cloned(),
            Some(conversation_id.clone()),
            "conversation session should have conversation_id set"
        );

        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        assert!(
            events.is_empty(),
            "expected zero events on a fresh conversation, got {}",
            events.len()
        );
    }

    #[tokio::test]
    async fn resume_conversation_applies_session_settings() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        // Wait for the initial session before closing so we don't race with
        // the spawn.
        let _initial = session_for_conversation(&state, &conversation_id).await;

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        let resumed = state
            .resume_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(resumed.item.status, ConversationStatus::Active);

        // The Resumed event records the new session id; use that to fetch the
        // newly-created session and confirm settings flowed through.
        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        assert_eq!(
            session.item.agent_config.model.as_deref(),
            Some("custom-model")
        );
    }

    #[tokio::test]
    async fn resume_conversation_sets_interactive_and_conversation_id() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        state
            .resume_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        assert!(
            session.item.is_interactive(),
            "resumed conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id().cloned(),
            Some(conversation_id),
            "resumed conversation session should have conversation_id set"
        );
    }

    #[tokio::test]
    async fn resume_conversation_sets_conversation_resume_from_to_event_count() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;

        // Send a couple of messages to add events.
        state
            .send_message(
                &conversation_id,
                "first".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        state
            .send_message(
                &conversation_id,
                "second".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        // The automation captures the resume snapshot at "index just after
        // the most recent Closed". With three UserMessages + a Closed in the
        // log, the expected value is exactly events.len() at this point.
        let events_before_resume = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let expected_resume_from = events_before_resume.len();

        state
            .resume_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        assert!(
            session.item.is_interactive(),
            "session should be interactive"
        );
        assert_eq!(
            session.item.mode.conversation_resume_from(),
            Some(expected_resume_from),
            "conversation_resume_from should equal index just after the most recent Closed"
        );
    }

    #[tokio::test]
    async fn send_message_from_active_appends_only_user_message() {
        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        // Wait for the initial spawn to settle before counting events.
        let _initial = session_for_conversation(&state, &conversation_id).await;
        let session_id = session_id_for_conversation(&state, &conversation_id).await;
        // Simulate the worker connecting so chat_relay flips to Active and
        // subsequent send_message calls dual-write to the session log
        // synchronously (drains any events queued during create_conversation).
        let _worker_rx = simulate_worker_connect(&state, &conversation_id, &session_id).await;

        let session_events_before = state.store().get_session_events(&session_id).await.unwrap();
        let count_before = session_events_before.len();

        state
            .send_message(
                &conversation_id,
                "second".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let events_after = poll_until(POLL_TIMEOUT, || async {
            let events = state.store().get_session_events(&session_id).await.unwrap();
            (events.len() > count_before).then_some(events)
        })
        .await
        .expect("expected the new UserMessage to be appended to the session log");
        let last = events_after.last().expect("expected at least one event");
        assert!(
            matches!(
                &last.item,
                DomainSessionEvent::UserMessage { content, .. } if content == "second"
            ),
            "expected the trailing event to be the new UserMessage, got {:?}",
            last.item
        );
        // No Resumed event on an already-Active conversation in the
        // conversation events log either.
        let conv_events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        assert!(
            !conv_events
                .iter()
                .any(|e| matches!(e.item, ConversationEvent::Resumed { .. })),
            "no Resumed event should be appended when conversation is already Active"
        );

        let versioned = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(versioned.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn send_message_from_closed_resumes_and_appends_user_message() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;
        let initial_session_id = session_id_for_conversation(&state, &conversation_id).await;

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        // Drive the prior session terminal so
        // `SpawnConversationSessionsAutomation` will spawn a fresh
        // resumed session rather than considering the initial one still
        // "active". In production the kill_job + monitor_running_sessions
        // flow drives this transition.
        //
        // The SessionUpdated event fired here drives the
        // SpawnConversationSessionsAutomation's idle-flip branch. While the
        // conversation is still Closed that branch is a no-op, so we sleep
        // briefly to let the automation drain the event before we flip the
        // conversation Active in `send_message`. Otherwise the stale
        // SessionUpdated could land *after* the Active flip and race the
        // conversation right back to Idle.
        mark_session_terminal(
            &state,
            &initial_session_id,
            crate::domain::task_status::Status::Complete,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        state
            .send_message(
                &conversation_id,
                "hello-again".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        // Wait for the resume-on-send to settle: the automation appends a
        // Resumed event on the conversation log and spawns a second session.
        // The new UserMessage lands on the new session's SessionEvent log
        // when the worker connects (set_active drains pending events into
        // the new session's log) — simulate that worker connection here.
        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        wait_for_session_count(&state, &conversation_id, 2).await;
        let _worker_rx =
            simulate_worker_connect(&state, &conversation_id, &resumed_session_id).await;

        let session_events = poll_until(POLL_TIMEOUT, || async {
            let events = state
                .store()
                .get_session_events(&resumed_session_id)
                .await
                .unwrap();
            events
                .iter()
                .any(|e| {
                    matches!(
                        &e.item,
                        DomainSessionEvent::UserMessage { content, .. } if content == "hello-again"
                    )
                })
                .then_some(events)
        })
        .await;
        assert!(
            session_events.is_some(),
            "expected the new UserMessage to be appended to the new session's SessionEvent log"
        );

        let versioned = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(versioned.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn send_message_from_closed_sets_conversation_resume_from() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;
        let settings = SessionSettings::default();

        let (conversation_id, _versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;
        let initial_session_id = session_id_for_conversation(&state, &conversation_id).await;

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        // Drive the prior session terminal — see the matching test above
        // for why this is required under the active-state-filter resolver,
        // and why we sleep briefly afterwards.
        mark_session_terminal(
            &state,
            &initial_session_id,
            crate::domain::task_status::Status::Complete,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // The automation captures the resume snapshot at the position just
        // after the most recent Closed event. We sample the event count here
        // — which currently equals the index after Closed — so the value
        // matches regardless of whether the racing UserMessage append lands
        // before or after the automation fires.
        let events_before_resume = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let expected_resume_from = events_before_resume.len();

        state
            .send_message(
                &conversation_id,
                "hello-again".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        assert!(
            session.item.is_interactive(),
            "session should be interactive"
        );
        assert_eq!(
            session.item.mode.conversation_resume_from(),
            Some(expected_resume_from),
            "conversation_resume_from should equal index just after the most recent Closed"
        );
    }

    #[tokio::test]
    async fn create_conversation_with_explicit_agent_name_applies_agent_prompt() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", false, vec![]).await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.resolved_prompt(), "you are an SWE");
    }

    #[tokio::test]
    async fn create_conversation_with_explicit_agent_applies_secrets_and_mcp_config() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt_and_mcp(
            &state,
            "swe",
            "you are an SWE",
            r#"{"mcpServers":{"foo":{"command":"foo"}}}"#,
            vec!["FOO".to_string()],
        )
        .await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.secrets, Some(vec!["FOO".to_string()]));
        assert!(
            session.item.agent_config.mcp_config.is_some(),
            "mcp_config should be set"
        );
    }

    #[tokio::test]
    async fn create_conversation_without_agent_uses_default_conversation_agent() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "default agent prompt", true, vec![]).await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.resolved_prompt(), "default agent prompt");
        assert_eq!(
            session
                .item
                .env_vars
                .get(AGENT_NAME_ENV_VAR)
                .map(String::as_str),
            Some("swe")
        );
    }

    #[tokio::test]
    async fn create_conversation_without_agent_and_no_default_does_not_spawn_session() {
        // With the spawn moved to `SpawnConversationSessionsAutomation`, a
        // conversation that has no `agent_name` and no registered default
        // conversation agent simply doesn't get a session — the automation
        // logs a warning and bails. This replaces the legacy behavior of
        // synchronously creating a session whose prompt was the bare user
        // message; that fallback is no longer reachable.
        //
        // Created with `message = None` so the bounded-wait resolver in
        // `send_message` is not exercised here — that path correctly errors
        // out when no active session ever appears, which is covered by its
        // own test below.
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);

        let (conversation_id, _) = state
            .create_conversation(
                None,
                None,
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        // Give the automation a moment to (not) spawn anything.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let sessions = state
            .store()
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        let matching = sessions
            .into_iter()
            .filter(|(_, s)| s.item.conversation_id() == Some(&conversation_id))
            .count();
        assert_eq!(
            matching, 0,
            "no session should be spawned for a conversation with no agent and no default"
        );
    }

    #[tokio::test]
    async fn create_conversation_with_unknown_agent_name_fails_with_agent_not_found() {
        // The spawn is asynchronous, but the request layer still validates
        // `agent_name` synchronously so client typos surface as a 4xx rather
        // than a silently-spawnless 200. Calls with `agent_name = None`
        // remain valid even when no default conversation agent is
        // registered (server-config concern, not client-input).
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);

        let result = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("does-not-exist").unwrap()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await;

        match result {
            Err(crate::app::CreateConversationError::AgentNotFound { name }) => {
                assert_eq!(name, "does-not-exist");
            }
            other => panic!(
                "expected AgentNotFound, got {:?}",
                other
                    .as_ref()
                    .err()
                    .map(|e| e.to_string())
                    .or_else(|| other.as_ref().ok().map(|_| "Ok".to_string()))
            ),
        }

        // Nothing should have been persisted: no conversation, no session.
        let sessions = state
            .store()
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert!(
            sessions.is_empty(),
            "no session should be spawned when agent_name validation fails"
        );
    }

    #[tokio::test]
    async fn create_conversation_merges_agent_secrets_with_session_settings_secrets() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(
            &state,
            "swe",
            "prompt",
            false,
            vec!["AGENT_SECRET".to_string(), "SHARED".to_string()],
        )
        .await;

        let settings = SessionSettings {
            secrets: Some(vec!["SESSION_SECRET".to_string(), "SHARED".to_string()]),
            ..Default::default()
        };

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap()),
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        // Agent secrets come first; shared secrets are deduped; order preserved.
        assert_eq!(
            session.item.secrets,
            Some(vec![
                "AGENT_SECRET".to_string(),
                "SHARED".to_string(),
                "SESSION_SECRET".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn resume_conversation_reapplies_agent_config() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt_and_mcp(
            &state,
            "swe",
            "agent prompt",
            r#"{"mcpServers":{}}"#,
            vec!["FOO".to_string()],
        )
        .await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        state
            .resume_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        assert_eq!(session.item.resolved_prompt(), "agent prompt");
        assert_eq!(session.item.secrets, Some(vec!["FOO".to_string()]));
        assert!(session.item.agent_config.mcp_config.is_some());
        assert_eq!(
            session
                .item
                .env_vars
                .get(AGENT_NAME_ENV_VAR)
                .map(String::as_str),
            Some("swe")
        );
    }

    #[tokio::test]
    async fn agent_name_env_var_present_on_conversation_session() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "prompt", false, vec![]).await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                Some(hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(
            session
                .item
                .env_vars
                .get(AGENT_NAME_ENV_VAR)
                .map(String::as_str),
            Some("swe")
        );
    }

    #[tokio::test]
    async fn send_message_as_creator_succeeds() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let _initial = session_for_conversation(&state, &conversation_id).await;
        let session_id = session_id_for_conversation(&state, &conversation_id).await;
        // Simulate the worker connecting so the dual-write on the next
        // send_message lands on the session log synchronously.
        let _worker_rx = simulate_worker_connect(&state, &conversation_id, &session_id).await;

        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        let session_events_before = state.store().get_session_events(&session_id).await.unwrap();
        let count_before = session_events_before.len();

        state
            .send_message(
                &conversation_id,
                "from-creator".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .expect("creator should be allowed to send a message");

        let events_after = poll_until(POLL_TIMEOUT, || async {
            let events = state.store().get_session_events(&session_id).await.unwrap();
            (events.len() > count_before).then_some(events)
        })
        .await
        .expect("expected the new UserMessage to be appended to the session log");
        let last = events_after.last().expect("expected at least one event");
        assert!(
            matches!(
                &last.item,
                DomainSessionEvent::UserMessage { content, .. } if content == "from-creator"
            ),
            "expected the trailing event to be the new UserMessage, got {:?}",
            last.item
        );
    }

    #[tokio::test]
    async fn send_message_as_non_creator_is_forbidden_and_does_not_mutate() {
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent_with_prompt(&state, "swe", "you are an SWE", true, vec![]).await;

        let (conversation_id, _) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        // Wait for the spawn to settle so the event log is in a stable state
        // before we assert on it.
        let _initial = session_for_conversation(&state, &conversation_id).await;

        let events_before = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let versioned_before = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();

        let result = state
            .send_message(
                &conversation_id,
                "intruder".to_string(),
                ActorRef::test(),
                Username::from("not-the-creator"),
            )
            .await;

        match result {
            Err(crate::app::SendMessageError::Forbidden { principal }) => {
                assert_eq!(principal, Username::from("not-the-creator"));
            }
            other => panic!(
                "expected Forbidden, got {:?}",
                other.as_ref().err().map(|e| e.to_string())
            ),
        }

        // Give a brief window for any (unintended) async work to surface,
        // then verify the conversation log and the conversation record are
        // unchanged.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let events_after = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        assert_eq!(
            events_after.len(),
            events_before.len(),
            "forbidden send_message must not append events",
        );
        // The forbidden caller's content must not appear on either log.
        let session_id = session_id_for_conversation(&state, &conversation_id).await;
        let session_events_after = state.store().get_session_events(&session_id).await.unwrap();
        use crate::domain::sessions::SessionEvent as DomainSessionEvent;
        assert!(
            !session_events_after.iter().any(|e| matches!(
                &e.item,
                DomainSessionEvent::UserMessage { content, .. } if content == "intruder"
            )),
            "intruder message must not be present in the session event log",
        );

        let versioned_after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(
            versioned_after.item, versioned_before.item,
            "forbidden send_message must not mutate the conversation",
        );
    }

    #[tokio::test]
    async fn send_message_with_no_session_queues_in_chat_relay() {
        // Under the queue-and-deliver model, send_message no longer
        // blocks on a session spawn; the event lands in the chat-relay
        // PendingConnection and is delivered atomically when a worker
        // connects. With no agent registered and no `agent_name` the
        // session never spawns, so we observe the queued shape: the
        // call returns Ok, and no UserMessage is written to any session
        // log (no session exists to write to).
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);

        let (conversation_id, _) = state
            .create_conversation(
                None,
                None,
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        state
            .send_message(
                &conversation_id,
                "hello".to_string(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .expect("send_message must accept the event for queueing");

        // No session was ever spawned, so no session log exists for the
        // message. The pending queue holds it in memory until a worker
        // connects (an acceptable tradeoff per the parent issue's brief).
        let sessions = state
            .store()
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert!(
            sessions.is_empty(),
            "no session should exist when no agent and no default are registered"
        );
    }
}
