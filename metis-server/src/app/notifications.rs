use crate::{
    domain::actors::ActorId,
    domain::notifications::Notification,
    store::{ReadOnlyStore, StoreError},
};
use chrono::{DateTime, Utc};
use metis_common::{NotificationId, api::v1::notifications::ListNotificationsQuery};
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("notification '{notification_id}' not found")]
    NotFound { notification_id: NotificationId },
    #[error("notification store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

const DEFAULT_LIMIT: u32 = 50;

impl AppState {
    /// List notifications for a recipient, applying query filters.
    pub async fn list_notifications(
        &self,
        recipient: &ActorId,
        query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, NotificationError> {
        let mut store_query = ListNotificationsQuery::default();
        store_query.recipient = Some(recipient.to_string());
        store_query.is_read = query.is_read;
        store_query.before = query.before;
        store_query.after = query.after;
        store_query.limit = Some(query.limit.unwrap_or(DEFAULT_LIMIT));

        self.store
            .list_notifications(&store_query)
            .await
            .map_err(|source| NotificationError::Store { source })
    }

    /// Count unread notifications for a recipient.
    pub async fn count_unread_notifications(
        &self,
        recipient: &ActorId,
    ) -> Result<u64, NotificationError> {
        self.store
            .count_unread_notifications(recipient)
            .await
            .map_err(|source| NotificationError::Store { source })
    }

    /// Mark a single notification as read, verifying that the actor owns it.
    pub async fn mark_notification_read(
        &self,
        actor: &ActorId,
        notification_id: &NotificationId,
    ) -> Result<(), NotificationError> {
        let notification = self
            .store
            .get_notification(notification_id)
            .await
            .map_err(|err| match err {
                StoreError::NotificationNotFound(_) => NotificationError::NotFound {
                    notification_id: notification_id.clone(),
                },
                other => NotificationError::Store { source: other },
            })?;

        if notification.recipient != *actor {
            return Err(NotificationError::NotFound {
                notification_id: notification_id.clone(),
            });
        }

        self.store
            .mark_notification_read(notification_id)
            .await
            .map_err(|err| match err {
                StoreError::NotificationNotFound(_) => NotificationError::NotFound {
                    notification_id: notification_id.clone(),
                },
                other => NotificationError::Store { source: other },
            })?;

        info!(notification_id = %notification_id, "notification marked as read");
        Ok(())
    }

    /// Mark all notifications as read for an actor, optionally filtering by timestamp.
    pub async fn mark_all_notifications_read(
        &self,
        actor: &ActorId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64, NotificationError> {
        self.store
            .mark_all_notifications_read(actor, before)
            .await
            .map_err(|source| NotificationError::Store { source })
    }
}
