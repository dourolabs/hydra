use crate::{
    domain::actors::{ActorId, ActorRef},
    domain::messages::Message,
    store::{ReadOnlyStore, StoreError},
};
use metis_common::{MessageId, VersionNumber, Versioned};
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("recipient not found: {actor_name}")]
    RecipientNotFound { actor_name: String },
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
        actor_ref: ActorRef,
    ) -> Result<(MessageId, VersionNumber, Versioned<Message>), SendMessageError> {
        let recipient_name = recipient.to_string();
        info!(sender = %sender, recipient = %recipient_name, "send_message invoked");

        // Validate recipient exists
        self.store.get_actor(&recipient_name).await.map_err(|_| {
            SendMessageError::RecipientNotFound {
                actor_name: recipient_name,
            }
        })?;

        let message = Message::new(Some(sender.clone()), recipient.clone(), body);

        let (message_id, _version) = self
            .store
            .add_message_with_actor(message, actor_ref)
            .await
            .map_err(|source| SendMessageError::Store { source })?;

        // Fetch the created message to get full versioned data
        let versioned = self
            .store
            .get_message(&message_id)
            .await
            .map_err(|source| SendMessageError::Store { source })?;

        info!(message_id = %message_id, "send_message completed");
        Ok((message_id, versioned.version, versioned))
    }
}
