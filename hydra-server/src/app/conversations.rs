use crate::{
    domain::{
        actors::ActorRef,
        conversations::{Conversation, ConversationEvent, ConversationStatus},
        users::Username,
    },
    store::StoreError,
};
use hydra_common::{
    ConversationId, Versioned,
    api::v1::sessions::{BundleSpec, CreateSessionRequest},
};
use std::collections::HashMap;
use thiserror::Error;
use tracing::info;

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

impl AppState {
    pub async fn create_conversation(
        &self,
        message: String,
        agent_name: Option<String>,
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
            deleted: false,
        };

        // 2. Persist the conversation
        let (conversation_id, _version) = self
            .store
            .add_conversation_with_actor(conversation.clone(), actor_ref.clone())
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        // 3. Append the first UserMessage event
        let event = ConversationEvent::UserMessage {
            content: message.clone(),
            timestamp: chrono::Utc::now(),
        };
        self.store
            .append_conversation_event_with_actor(&conversation_id, event, actor_ref.clone())
            .await
            .map_err(|source| CreateConversationError::Store { source })?;

        // 4. Create an interactive session
        let session_request = CreateSessionRequest::new(
            message,
            None,
            BundleSpec::None,
            HashMap::new(),
            None,
            true,
        );
        let session_id = self
            .create_session(session_request, actor_ref.clone(), creator)
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
}
