use super::actors::ActorId;
use crate::app::event_bus::MutationPayload;
use crate::store::{ReadOnlyStore, StoreError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, MetisId, VersionNumber};
use serde::{Deserialize, Serialize};

/// The server-side domain notification type.
///
/// Notifications are non-versioned: the only mutation after creation is marking as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub recipient: ActorId,
    pub source_actor: Option<ActorId>,
    pub object_kind: String,
    pub object_id: MetisId,
    pub object_version: VersionNumber,
    pub event_type: String,
    pub summary: String,
    pub source_issue_id: Option<IssueId>,
    pub policy: String,
    pub is_read: bool,
    pub created_at: DateTime<Utc>,
}

impl Notification {
    pub fn new(
        recipient: ActorId,
        source_actor: Option<ActorId>,
        object_kind: String,
        object_id: MetisId,
        object_version: VersionNumber,
        event_type: String,
        summary: String,
        source_issue_id: Option<IssueId>,
        policy: String,
    ) -> Self {
        Self {
            recipient,
            source_actor,
            object_kind,
            object_id,
            object_version,
            event_type,
            summary,
            source_issue_id,
            policy,
            is_read: false,
            created_at: Utc::now(),
        }
    }
}

// Conversions between domain and API wire types.
use metis_common::api::v1 as api;

impl From<api::notifications::Notification> for Notification {
    fn from(value: api::notifications::Notification) -> Self {
        Self {
            recipient: value.recipient,
            source_actor: value.source_actor,
            object_kind: value.object_kind,
            object_id: value.object_id,
            object_version: value.object_version,
            event_type: value.event_type,
            summary: value.summary,
            source_issue_id: value.source_issue_id,
            policy: value.policy,
            is_read: value.is_read,
            created_at: value.created_at,
        }
    }
}

impl From<Notification> for api::notifications::Notification {
    fn from(value: Notification) -> Self {
        let mut notif = api::notifications::Notification::new(
            value.recipient,
            value.source_actor,
            value.object_kind,
            value.object_id,
            value.object_version,
            value.event_type,
            value.summary,
            value.source_issue_id,
            value.policy,
        );
        notif.is_read = value.is_read;
        notif.created_at = value.created_at;
        notif
    }
}

/// A policy that determines who should be notified about a given event.
///
/// The `NotificationWorker` holds a `Vec<Box<dyn NotificationPolicy>>` and runs all
/// policies for each event, deduplicating recipients across policies. Each policy is
/// independently testable and produces a set of `ActorId`s that should receive a
/// notification for the given mutation.
///
/// The `policy` column in the notifications table records which policy generated each
/// row, enabling debugging and future per-policy management.
#[async_trait]
pub trait NotificationPolicy: Send + Sync {
    /// A short identifier for this policy, stored in the `policy` column.
    fn name(&self) -> &str;

    /// Given a mutation event and read-only access to the store, return the set of
    /// `ActorId`s that should receive a notification.
    ///
    /// The caller is responsible for deduplicating and filtering (e.g., removing the
    /// actor who caused the change).
    async fn resolve_recipients(
        &self,
        event: &MutationPayload,
        store: &dyn ReadOnlyStore,
    ) -> Result<Vec<ActorId>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::users::Username;
    use metis_common::IssueId;

    #[test]
    fn notification_domain_roundtrip() {
        let notif = Notification::new(
            ActorId::Username(Username::from("alice").into()),
            Some(ActorId::Issue("i-abcdef".parse::<IssueId>().unwrap())),
            "issue".to_string(),
            "i-abcdef".parse::<IssueId>().unwrap().into(),
            1,
            "updated".to_string(),
            "Issue status changed".to_string(),
            None,
            "walk_up".to_string(),
        );

        let json = serde_json::to_string(&notif).expect("serialize");
        let decoded: Notification = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, notif);
        assert!(!decoded.is_read);
    }

    #[test]
    fn notification_api_domain_roundtrip() {
        let api_notif = api::notifications::Notification::new(
            ActorId::Username(Username::from("alice").into()),
            None,
            "patch".to_string(),
            metis_common::PatchId::new().into(),
            1,
            "created".to_string(),
            "Patch created".to_string(),
            None,
            "walk_up".to_string(),
        );

        let domain_notif: Notification = api_notif.clone().into();
        let back: api::notifications::Notification = domain_notif.into();
        assert_eq!(back, api_notif);
    }
}
