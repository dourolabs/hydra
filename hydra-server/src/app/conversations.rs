use crate::{
    app::chat_relay,
    domain::{
        actors::ActorRef,
        conversations::{Conversation, ConversationEvent, ConversationStatus},
        users::Username,
    },
    store::StoreError,
};
use hydra_common::{ConversationId, Versioned, api::v1::conversations as api_conversations};
use thiserror::Error;
use tracing::{info, warn};

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum CreateConversationError {
    #[error("failed to store conversation")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("failed to access conversation store")]
    Store {
        #[source]
        source: StoreError,
    },
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
        agent_name: Option<String>,
        session_settings: crate::domain::issues::SessionSettings,
        actor_ref: ActorRef,
        creator: Username,
    ) -> Result<(ConversationId, Versioned<Conversation>), CreateConversationError> {
        // Persist the conversation in Active status. The companion session
        // is spawned asynchronously by `SpawnConversationSessionsAutomation`
        // (in `policy/automations/spawn_conversation_sessions.rs`) when the
        // ConversationCreated event lands on the bus.
        let conversation = Conversation {
            title: None,
            agent_name,
            status: ConversationStatus::Active,
            creator,
            session_settings,
            deleted: false,
        };

        let (conversation_id, _version) = self
            .store
            .add_conversation_with_actor(conversation, actor_ref.clone())
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        if let Some(content) = message {
            let event = ConversationEvent::UserMessage {
                content,
                timestamp: chrono::Utc::now(),
            };
            self.store
                .append_conversation_event_with_actor(&conversation_id, event, actor_ref)
                .await
                .map_err(|source| CreateConversationError::Store { source })?;
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
    ) -> Result<api_conversations::ConversationEvent, SendMessageError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| SendMessageError::Store { source })?;

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

        // Append UserMessage event
        let event = ConversationEvent::UserMessage {
            content,
            timestamp: chrono::Utc::now(),
        };
        self.store
            .append_conversation_event_with_actor(conversation_id, event.clone(), actor_ref)
            .await
            .map_err(|source| SendMessageError::Store { source })?;

        // Forward to worker via ChatRelayMap if connected
        let api_event: api_conversations::ConversationEvent = event.into();
        match chat_relay::send_to_worker(&self.chat_relay_map, conversation_id, api_event.clone())
            .await
        {
            Ok(()) => {
                info!(conversation_id = %conversation_id, "message forwarded to worker");
            }
            Err(chat_relay::SendToWorkerError::NoRelay) => {
                info!(conversation_id = %conversation_id, "no relay connected, worker will catch up");
            }
            Err(err) => {
                warn!(conversation_id = %conversation_id, error = %err, "failed to forward message to worker");
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
            .append_conversation_event_with_actor(conversation_id, event, actor_ref.clone())
            .await
            .map_err(|source| CloseConversationError::Store { source })?;

        // Kill the worker if one is currently relaying this conversation.
        // If no entry is present, no worker is connected and no kill is needed.
        if let Some(entry) = self.chat_relay_map.get(conversation_id).map(|e| e.clone()) {
            let session_id = entry.session_id;
            match self.job_engine.kill_job(&session_id).await {
                Ok(()) => {
                    info!(conversation_id = %conversation_id, session_id = %session_id, "killed active session");
                }
                Err(err) => {
                    warn!(conversation_id = %conversation_id, session_id = %session_id, error = %err, "failed to kill session (may already be stopped)");
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
    use hydra_common::{ConversationId, Versioned, api::v1::sessions::SearchSessionsQuery};
    use std::time::Duration;

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
            created_by: None,
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
            created_by: None,
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
            created_by: None,
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
        assert_eq!(session.item.model.as_deref(), Some("custom-model"));
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
        assert_eq!(session.item.model.as_deref(), Some("default-model"));
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
        match &session.item.context {
            crate::domain::sessions::BundleSpec::GitRepository { url, rev } => {
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
        assert_eq!(session.item.model.as_deref(), Some("custom-model"));
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
            .send_message(&conversation_id, "first".to_string(), ActorRef::test())
            .await
            .unwrap();
        state
            .send_message(&conversation_id, "second".to_string(), ActorRef::test())
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
        let opts = session
            .item
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(
            opts.conversation_resume_from,
            Some(expected_resume_from),
            "conversation_resume_from should equal index just after the most recent Closed"
        );
    }

    #[tokio::test]
    async fn send_message_from_active_appends_only_user_message() {
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

        let events_before = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let count_before = events_before.len();

        state
            .send_message(&conversation_id, "second".to_string(), ActorRef::test())
            .await
            .unwrap();

        // Give any racing automation a moment to mis-fire, then assert that
        // the event log only grew by the single UserMessage send_message
        // appended (no Resumed event on an already-Active conversation).
        let events_after = poll_until(POLL_TIMEOUT, || async {
            let events = state
                .store()
                .get_conversation_events(&conversation_id)
                .await
                .unwrap();
            (events.len() > count_before).then_some(events)
        })
        .await
        .expect("expected the new UserMessage to be appended");
        assert_eq!(
            events_after.len(),
            count_before + 1,
            "send_message on an Active conversation should append exactly one event"
        );
        let last = events_after.last().expect("expected at least one event");
        assert!(
            matches!(
                last.item,
                ConversationEvent::UserMessage { ref content, .. } if content == "second"
            ),
            "expected the trailing event to be the new UserMessage, got {:?}",
            last.item
        );
        assert!(
            !events_after
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

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        state
            .send_message(
                &conversation_id,
                "hello-again".to_string(),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Wait for the resume-on-send to settle: the automation appends a
        // Resumed event and spawns a second session. NOTE: because the
        // automation is asynchronous, the *order* of Resumed vs the new
        // UserMessage in the event log is no longer guaranteed (it depends on
        // whether the automation processes the status flip before or after
        // send_message appends the UserMessage). The test only verifies that
        // both events end up in the log.
        let _resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        wait_for_session_count(&state, &conversation_id, 2).await;

        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let has_new_user_msg = events.iter().any(|e| {
            matches!(
                &e.item,
                ConversationEvent::UserMessage { content, .. } if content == "hello-again"
            )
        });
        assert!(
            has_new_user_msg,
            "expected the new UserMessage to be appended after resume-on-send"
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

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

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
            )
            .await
            .unwrap();

        let resumed_session_id = wait_for_resumed_event(&state, &conversation_id).await;
        let session = state
            .store()
            .get_session(&resumed_session_id, false)
            .await
            .unwrap();
        let opts = session
            .item
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(
            opts.conversation_resume_from,
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
                Some("swe".to_string()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.prompt, "you are an SWE");
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
                Some("swe".to_string()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session = session_for_conversation(&state, &conversation_id).await;
        assert_eq!(session.item.secrets, Some(vec!["FOO".to_string()]));
        assert!(
            session.item.mcp_config.is_some(),
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
        assert_eq!(session.item.prompt, "default agent prompt");
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
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);

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
    async fn create_conversation_with_unknown_agent_name_does_not_spawn_session() {
        // With the spawn moved to the automation, an unknown agent_name no
        // longer fails the request — `create_conversation` succeeds and the
        // automation logs a warning when it can't resolve the agent. This is
        // the documented behavior change called out in the issue tracker;
        // callers that need synchronous validation should look the agent up
        // before calling `create_conversation`.
        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);

        let (conversation_id, versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                Some("does-not-exist".to_string()),
                SessionSettings::default(),
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .expect("create_conversation should succeed even with unknown agent");
        assert_eq!(versioned.item.status, ConversationStatus::Active);

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
            "unknown agent should leave the conversation without a spawned session"
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
                Some("swe".to_string()),
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
                Some("swe".to_string()),
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
        assert_eq!(session.item.prompt, "agent prompt");
        assert_eq!(session.item.secrets, Some(vec!["FOO".to_string()]));
        assert!(session.item.mcp_config.is_some());
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
                Some("swe".to_string()),
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
}
