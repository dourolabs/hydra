use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Standard HTTP header name for SSE reconnection support.
pub const LAST_EVENT_ID_HEADER: &str = "last-event-id";

/// Query parameters for the GET /v1/events SSE endpoint.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct EventsQuery {
    /// Comma-separated entity types to filter (e.g. "issues,jobs").
    #[serde(default)]
    pub types: Option<String>,

    /// Comma-separated issue IDs to filter.
    #[serde(default)]
    pub issue_ids: Option<String>,

    /// Comma-separated job IDs to filter.
    #[serde(default)]
    pub job_ids: Option<String>,

    /// Comma-separated patch IDs to filter.
    #[serde(default)]
    pub patch_ids: Option<String>,

    /// Comma-separated document IDs to filter.
    #[serde(default)]
    pub document_ids: Option<String>,

    /// Comma-separated message IDs to filter.
    #[serde(default)]
    pub message_ids: Option<String>,

    /// Comma-separated notification IDs to filter.
    #[serde(default)]
    pub notification_ids: Option<String>,
}

impl EventsQuery {
    /// Build query string key-value pairs for URL construction.
    pub fn query_pairs(&self) -> Vec<(&str, String)> {
        let mut params = Vec::new();
        if let Some(ref types) = self.types {
            params.push(("types", types.clone()));
        }
        if let Some(ref ids) = self.issue_ids {
            params.push(("issue_ids", ids.clone()));
        }
        if let Some(ref ids) = self.job_ids {
            params.push(("job_ids", ids.clone()));
        }
        if let Some(ref ids) = self.patch_ids {
            params.push(("patch_ids", ids.clone()));
        }
        if let Some(ref ids) = self.document_ids {
            params.push(("document_ids", ids.clone()));
        }
        if let Some(ref ids) = self.message_ids {
            params.push(("message_ids", ids.clone()));
        }
        if let Some(ref ids) = self.notification_ids {
            params.push(("notification_ids", ids.clone()));
        }
        params
    }
}

/// The SSE event type names sent in the `event:` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SseEventType {
    IssueCreated,
    IssueUpdated,
    IssueDeleted,
    PatchCreated,
    PatchUpdated,
    PatchDeleted,
    JobCreated,
    JobUpdated,
    DocumentCreated,
    DocumentUpdated,
    DocumentDeleted,
    MessageCreated,
    MessageUpdated,
    NotificationCreated,
    Snapshot,
    Resync,
    Heartbeat,
}

impl SseEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IssueCreated => "issue_created",
            Self::IssueUpdated => "issue_updated",
            Self::IssueDeleted => "issue_deleted",
            Self::PatchCreated => "patch_created",
            Self::PatchUpdated => "patch_updated",
            Self::PatchDeleted => "patch_deleted",
            Self::JobCreated => "job_created",
            Self::JobUpdated => "job_updated",
            Self::DocumentCreated => "document_created",
            Self::DocumentUpdated => "document_updated",
            Self::DocumentDeleted => "document_deleted",
            Self::MessageCreated => "message_created",
            Self::MessageUpdated => "message_updated",
            Self::NotificationCreated => "notification_created",
            Self::Snapshot => "snapshot",
            Self::Resync => "resync",
            Self::Heartbeat => "heartbeat",
        }
    }
}

impl std::str::FromStr for SseEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "issue_created" => Ok(Self::IssueCreated),
            "issue_updated" => Ok(Self::IssueUpdated),
            "issue_deleted" => Ok(Self::IssueDeleted),
            "patch_created" => Ok(Self::PatchCreated),
            "patch_updated" => Ok(Self::PatchUpdated),
            "patch_deleted" => Ok(Self::PatchDeleted),
            "job_created" => Ok(Self::JobCreated),
            "job_updated" => Ok(Self::JobUpdated),
            "document_created" => Ok(Self::DocumentCreated),
            "document_updated" => Ok(Self::DocumentUpdated),
            "document_deleted" => Ok(Self::DocumentDeleted),
            "message_created" => Ok(Self::MessageCreated),
            "message_updated" => Ok(Self::MessageUpdated),
            "notification_created" => Ok(Self::NotificationCreated),
            "snapshot" => Ok(Self::Snapshot),
            "resync" => Ok(Self::Resync),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown SSE event type: {other}")),
        }
    }
}

/// Data payload for entity mutation events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct EntityEventData {
    pub entity_type: String,
    pub entity_id: String,
    pub version: u64,
    pub timestamp: DateTime<Utc>,
    /// Full entity state after the mutation, serialized as a version record
    /// (e.g., `IssueVersionRecord`, `JobVersionRecord`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<serde_json::Value>,
}

/// Data payload for the snapshot event sent on initial connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SnapshotEventData {
    /// Map from entity ID to its current version number.
    pub versions: HashMap<String, u64>,
}

/// Data payload for the resync event sent when the client has fallen behind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ResyncEventData {
    pub reason: String,
    pub current_seq: u64,
}

/// Data payload for heartbeat events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct HeartbeatEventData {
    pub server_time: DateTime<Utc>,
}
