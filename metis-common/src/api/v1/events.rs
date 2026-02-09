use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Query parameters for the GET /v1/events SSE endpoint.
#[derive(Debug, Default, Serialize, Deserialize)]
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
}

/// The SSE event type names sent in the `event:` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
            Self::Snapshot => "snapshot",
            Self::Resync => "resync",
            Self::Heartbeat => "heartbeat",
        }
    }
}

/// Data payload for entity mutation events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEventData {
    pub entity_type: String,
    pub entity_id: String,
    pub timestamp: DateTime<Utc>,
}

/// Data payload for the snapshot event sent on initial connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEventData {
    /// Map from entity ID to its current version number.
    pub versions: HashMap<String, u64>,
}

/// Data payload for the resync event sent when the client has fallen behind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResyncEventData {
    pub reason: String,
    pub current_seq: u64,
}

/// Data payload for heartbeat events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEventData {
    pub server_time: DateTime<Utc>,
}
