use crate::{
    domain::actors::{ActorId, ActorRef},
    domain::messages::Message,
    store::{ReadOnlyStore, StoreError},
};
use hydra_common::{MessageId, VersionNumber, Versioned, api::v1::messages::SearchMessagesQuery};
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum MessageError {
    #[error("recipient not found: {actor_name}")]
    RecipientNotFound { actor_name: String },
    #[error("message '{message_id}' not found")]
    NotFound { message_id: MessageId },
    #[error("message store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    /// Send a message from the authenticated actor to a recipient.
    ///
    /// Validates that the recipient exists, creates the message, and stores it
    /// (emitting a MessageCreated event).
    pub async fn send_message(
        &self,
        sender: &ActorId,
        recipient: &ActorId,
        body: String,
        is_read: bool,
        actor_ref: ActorRef,
    ) -> Result<(MessageId, VersionNumber, Versioned<Message>), MessageError> {
        let recipient_name = recipient.to_string();
        info!(sender = %sender, recipient = %recipient_name, "send_message invoked");

        // Validate recipient exists
        self.store.get_actor(&recipient_name).await.map_err(|_| {
            MessageError::RecipientNotFound {
                actor_name: recipient_name,
            }
        })?;

        let mut message = Message::new(Some(sender.clone()), recipient.clone(), body);
        message.is_read = is_read;

        let (message_id, _version) = self
            .store
            .add_message_with_actor(message, actor_ref)
            .await
            .map_err(|source| MessageError::Store { source })?;

        // Fetch the created message to get full versioned data
        let versioned = self
            .store
            .get_message(&message_id)
            .await
            .map_err(|source| MessageError::Store { source })?;

        info!(message_id = %message_id, "send_message completed");
        Ok((message_id, versioned.version, versioned))
    }

    /// List messages matching the given query filters.
    pub async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, MessageError> {
        self.store
            .list_messages(query)
            .await
            .map_err(|source| MessageError::Store { source })
    }

    /// Get a single message by ID.
    pub async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, MessageError> {
        self.store.get_message(id).await.map_err(|err| match err {
            StoreError::MessageNotFound(_) => MessageError::NotFound {
                message_id: id.clone(),
            },
            other => MessageError::Store { source: other },
        })
    }

    /// Mark a message as read, returning the new version number.
    pub async fn mark_message_read(
        &self,
        id: &MessageId,
        actor: ActorRef,
    ) -> Result<VersionNumber, MessageError> {
        let msg = self.get_message(id).await?;

        if msg.item.is_read {
            return Ok(msg.version);
        }

        let mut updated = msg.item.clone();
        updated.is_read = true;

        self.store
            .update_message_with_actor(id, updated, actor)
            .await
            .map_err(|source| MessageError::Store { source })
    }
}
