use crate::actor_ref::ActorId;
use crate::{IssueId, MetisId, NotificationId, VersionNumber};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A notification record representing an event that a recipient should be aware of.
///
/// Notifications are non-versioned: the only mutation after creation is marking as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Notification {
    pub recipient: ActorId,
    pub source_actor: Option<ActorId>,
    pub object_kind: String,
    pub object_id: MetisId,
    pub object_version: VersionNumber,
    pub event_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_issue_id: Option<IssueId>,
    pub policy: String,
    #[serde(default)]
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

/// Response containing a single notification with its ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct NotificationResponse {
    pub notification_id: NotificationId,
    pub notification: Notification,
}

impl NotificationResponse {
    pub fn new(notification_id: NotificationId, notification: Notification) -> Self {
        Self {
            notification_id,
            notification,
        }
    }
}

/// Query parameters for listing notifications.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListNotificationsQuery {
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub is_read: Option<bool>,
    #[serde(default)]
    pub before: Option<DateTime<Utc>>,
    #[serde(default)]
    pub after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Response containing a list of notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListNotificationsResponse {
    pub notifications: Vec<NotificationResponse>,
    pub has_more: bool,
}

impl ListNotificationsResponse {
    pub fn new(notifications: Vec<NotificationResponse>, has_more: bool) -> Self {
        Self {
            notifications,
            has_more,
        }
    }
}

/// Response containing the count of unread notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UnreadCountResponse {
    pub count: u64,
}

impl UnreadCountResponse {
    pub fn new(count: u64) -> Self {
        Self { count }
    }
}

/// Response after marking notifications as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct MarkReadResponse {
    pub marked: u64,
}

impl MarkReadResponse {
    pub fn new(marked: u64) -> Self {
        Self { marked }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::users::Username;

    #[test]
    fn notification_serde_round_trip() {
        let notif = Notification::new(
            ActorId::Username(Username::from("alice")),
            Some(ActorId::Issue(crate::IssueId::new())),
            "issue".to_string(),
            crate::IssueId::new().into(),
            1,
            "updated".to_string(),
            "Issue status changed to in-progress".to_string(),
            None,
            "walk_up".to_string(),
        );

        let json = serde_json::to_string(&notif).expect("serialize");
        let decoded: Notification = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, notif);
    }

    #[test]
    fn notification_defaults() {
        let notif = Notification::new(
            ActorId::Username(Username::from("bob")),
            None,
            "patch".to_string(),
            crate::PatchId::new().into(),
            2,
            "created".to_string(),
            "Patch created".to_string(),
            Some(crate::IssueId::new()),
            "walk_up".to_string(),
        );
        assert!(!notif.is_read);
    }

    #[test]
    fn list_notifications_query_defaults() {
        let query = ListNotificationsQuery::default();
        assert_eq!(query.recipient, None);
        assert_eq!(query.is_read, None);
        assert_eq!(query.before, None);
        assert_eq!(query.after, None);
        assert_eq!(query.limit, None);
    }

    #[test]
    fn unread_count_response_serde() {
        let resp = UnreadCountResponse::new(42);
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: UnreadCountResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.count, 42);
    }

    #[test]
    fn mark_read_response_serde() {
        let resp = MarkReadResponse::new(5);
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: MarkReadResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.marked, 5);
    }

    #[test]
    fn list_notifications_response_has_more_true() {
        let resp = ListNotificationsResponse::new(vec![], true);
        assert!(resp.has_more);
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: ListNotificationsResponse = serde_json::from_str(&json).expect("deserialize");
        assert!(decoded.has_more);
    }

    #[test]
    fn list_notifications_response_has_more_false() {
        let resp = ListNotificationsResponse::new(vec![], false);
        assert!(!resp.has_more);
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: ListNotificationsResponse = serde_json::from_str(&json).expect("deserialize");
        assert!(!decoded.has_more);
    }

    #[test]
    fn list_notifications_response_backwards_compat() {
        // Responses without has_more (e.g., from older servers) should fail deserialization,
        // since has_more is a required field.
        let json = r#"{"notifications":[]}"#;
        let result = serde_json::from_str::<ListNotificationsResponse>(json);
        assert!(result.is_err());
    }
}
