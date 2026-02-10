use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Standard HTTP header name for SSE reconnection support.
pub const LAST_EVENT_ID_HEADER: &str = "last-event-id";

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
        params
    }
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
            "snapshot" => Ok(Self::Snapshot),
            "resync" => Ok(Self::Resync),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown SSE event type: {other}")),
        }
    }
}

/// Data payload for entity mutation events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEventData {
    pub entity_type: String,
    pub entity_id: String,
    pub version: u64,
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
