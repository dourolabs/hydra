use crate::{
    app::chat_relay,
    domain::{
        actors::ActorRef,
        conversations::{Conversation, ConversationEvent, ConversationStatus},
        users::Username,
    },
    store::StoreError,
};
use hydra_common::{
    ConversationId, Versioned,
    api::v1::{
        conversations as api_conversations,
        sessions::{BundleSpec, CreateSessionRequest},
    },
};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};

use super::{CreateSessionError, app_state::AppState};

#[derive(Debug, Error)]
pub enum CreateConversationError {
    #[error("failed to store conversation")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("failed to create session for conversation")]
    Session {
        #[source]
        source: CreateSessionError,
    },
}

#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("failed to access conversation store")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("conversation is not active (status: {status:?})")]
    NotActive { status: ConversationStatus },
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
    #[error("failed to create session for conversation")]
    Session {
        #[source]
        source: CreateSessionError,
    },
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
        // 1. Create a domain Conversation with status Active
        let conversation = Conversation {
            title: None,
            agent_name,
            active_session_id: None,
            status: ConversationStatus::Active,
            creator: creator.clone(),
            session_settings,
            deleted: false,
        };

        // 2. Persist the conversation
        let (conversation_id, _version) = self
            .store
            .add_conversation_with_actor(conversation.clone(), actor_ref.clone())
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        // 3. Append the first UserMessage event if a message was provided
        if let Some(content) = message.as_ref() {
            let event = ConversationEvent::UserMessage {
                content: content.clone(),
                timestamp: chrono::Utc::now(),
            };
            self.store
                .append_conversation_event_with_actor(&conversation_id, event, actor_ref.clone())
                .await
                .map_err(|source| CreateConversationError::Store { source })?;
        }

        // 4. Create an interactive session, applying conversation session_settings
        let session_request =
            CreateSessionRequest::new(message, None, BundleSpec::None, HashMap::new(), None, true);
        let settings = conversation.session_settings.clone();
        let session_id = self
            .create_session(
                session_request,
                Some(settings),
                actor_ref.clone(),
                creator,
                Some(conversation_id.clone()),
            )
            .await
            .map_err(|source| CreateConversationError::Session { source })?;

        // 5. Update conversation with active_session_id
        let mut updated_conversation = conversation;
        updated_conversation.active_session_id = Some(session_id);
        self.store
            .update_conversation_with_actor(&conversation_id, updated_conversation, actor_ref)
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        // 6. Fetch and return the final conversation state
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

        // Verify conversation is Active
        if versioned.item.status != ConversationStatus::Active {
            return Err(SendMessageError::NotActive {
                status: versioned.item.status,
            });
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
        if let Some(session_id) = &versioned.item.active_session_id {
            let api_event: api_conversations::ConversationEvent = event.clone().into();
            match chat_relay::send_to_worker(&self.chat_relay_map, session_id, api_event).await {
                Ok(()) => {
                    info!(conversation_id = %conversation_id, session_id = %session_id, "message forwarded to worker");
                }
                Err(chat_relay::SendToWorkerError::NoRelay) => {
                    info!(conversation_id = %conversation_id, session_id = %session_id, "no relay connected, worker will catch up");
                }
                Err(err) => {
                    warn!(conversation_id = %conversation_id, session_id = %session_id, error = %err, "failed to forward message to worker");
                }
            }
        }

        let api_event: api_conversations::ConversationEvent = event.into();
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

        // Kill session if active
        if let Some(session_id) = &versioned.item.active_session_id {
            match self.job_engine.kill_job(session_id).await {
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
        updated.active_session_id = None;
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
        creator: Username,
    ) -> Result<Versioned<Conversation>, ResumeConversationError> {
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        // Verify not already Active
        if versioned.item.status == ConversationStatus::Active {
            return Err(ResumeConversationError::AlreadyActive);
        }

        // Create a new interactive session, applying conversation session_settings
        let session_request =
            CreateSessionRequest::new(None, None, BundleSpec::None, HashMap::new(), None, true);
        let settings = versioned.item.session_settings.clone();
        let session_id = self
            .create_session(
                session_request,
                Some(settings),
                actor_ref.clone(),
                creator,
                Some(conversation_id.clone()),
            )
            .await
            .map_err(|source| ResumeConversationError::Session { source })?;

        // Append Resumed event
        let event = ConversationEvent::Resumed {
            session_id: session_id.clone(),
            timestamp: chrono::Utc::now(),
        };
        self.store
            .append_conversation_event_with_actor(conversation_id, event, actor_ref.clone())
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        // Update conversation status
        let mut updated = versioned.item;
        updated.status = ConversationStatus::Active;
        updated.active_session_id = Some(session_id.clone());
        self.store
            .update_conversation_with_actor(conversation_id, updated, actor_ref)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        // Return updated conversation
        let versioned = self
            .store()
            .get_conversation(conversation_id, false)
            .await
            .map_err(|source| ResumeConversationError::Store { source })?;

        info!(conversation_id = %conversation_id, session_id = %session_id, "conversation resumed");
        Ok(versioned)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::test_helpers::state_with_default_model,
        domain::{
            actors::ActorRef, conversations::ConversationStatus, issues::SessionSettings,
            users::Username,
        },
    };

    #[tokio::test]
    async fn create_conversation_applies_session_settings_model() {
        let state = state_with_default_model("default-model");
        let settings = SessionSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let (_conversation_id, versioned) = state
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
        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert_eq!(session.item.model.as_deref(), Some("custom-model"));
    }

    #[tokio::test]
    async fn create_conversation_applies_default_model_from_config() {
        let state = state_with_default_model("default-model");
        let settings = SessionSettings::default();

        let (_conversation_id, versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert_eq!(session.item.model.as_deref(), Some("default-model"));
    }

    #[tokio::test]
    async fn create_conversation_applies_remote_url_to_context() {
        let state = state_with_default_model("default-model");
        let settings = SessionSettings {
            remote_url: Some("https://github.com/org/repo.git".to_string()),
            branch: Some("feature".to_string()),
            ..Default::default()
        };

        let (_conversation_id, versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
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
        let settings = SessionSettings {
            secrets: Some(vec!["GH_TOKEN".to_string()]),
            ..Default::default()
        };

        let (_conversation_id, versioned) = state
            .create_conversation(
                Some("hello".to_string()),
                None,
                settings,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert_eq!(session.item.secrets, Some(vec!["GH_TOKEN".to_string()]));
    }

    #[tokio::test]
    async fn create_conversation_sets_interactive_and_conversation_id() {
        let state = state_with_default_model("default-model");
        let settings = SessionSettings::default();

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

        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert!(
            session.item.interactive,
            "conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id,
            Some(conversation_id),
            "conversation session should have conversation_id set"
        );
    }

    #[tokio::test]
    async fn create_conversation_with_no_message_starts_with_zero_events() {
        let state = state_with_default_model("default-model");
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

        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        assert!(
            events.is_empty(),
            "expected zero events, got {}",
            events.len()
        );

        let session_id = versioned.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert!(
            session.item.interactive,
            "conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id,
            Some(conversation_id),
            "conversation session should have conversation_id set"
        );
    }

    #[tokio::test]
    async fn resume_conversation_applies_session_settings() {
        let state = state_with_default_model("default-model");
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

        // Close the conversation
        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        // Resume and verify settings are applied to the new session
        let resumed = state
            .resume_conversation(
                &conversation_id,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        assert_eq!(resumed.item.status, ConversationStatus::Active);
        let session_id = resumed.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert_eq!(session.item.model.as_deref(), Some("custom-model"));
    }

    #[tokio::test]
    async fn resume_conversation_sets_interactive_and_conversation_id() {
        let state = state_with_default_model("default-model");
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

        state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .unwrap();

        let resumed = state
            .resume_conversation(
                &conversation_id,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();

        let session_id = resumed.item.active_session_id.as_ref().unwrap();
        let session = state.store().get_session(session_id, false).await.unwrap();
        assert!(
            session.item.interactive,
            "resumed conversation session should be interactive"
        );
        assert_eq!(
            session.item.conversation_id,
            Some(conversation_id),
            "resumed conversation session should have conversation_id set"
        );
    }
}
