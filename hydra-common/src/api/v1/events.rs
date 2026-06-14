use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

    /// Comma-separated session IDs to filter.
    #[serde(default, alias = "job_ids")]
    pub session_ids: Option<String>,

    /// Comma-separated patch IDs to filter.
    #[serde(default)]
    pub patch_ids: Option<String>,

    /// Comma-separated label IDs to filter.
    #[serde(default)]
    pub label_ids: Option<String>,

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
        if let Some(ref ids) = self.session_ids {
            params.push(("session_ids", ids.clone()));
        }
        if let Some(ref ids) = self.patch_ids {
            params.push(("patch_ids", ids.clone()));
        }
        if let Some(ref ids) = self.label_ids {
            params.push(("label_ids", ids.clone()));
        }
        if let Some(ref ids) = self.document_ids {
            params.push(("document_ids", ids.clone()));
        }
        params
    }
}

/// The SSE event type names sent in the `event:` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
// wire-casing-exempt: published SSE `event:` strings (e.g. `issue_created`) are consumed by hydra-web and CLI watchers; coordinated rename tracked under parent issue i-glwrexmf.
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SseEventType {
    IssueCreated,
    IssueUpdated,
    IssueDeleted,
    PatchCreated,
    PatchUpdated,
    PatchDeleted,
    SessionCreated,
    SessionUpdated,
    DocumentCreated,
    DocumentUpdated,
    DocumentDeleted,
    LabelCreated,
    LabelUpdated,
    LabelDeleted,
    ConversationCreated,
    ConversationUpdated,
    SessionEventCreated,
    SessionStateUpdated,
    SessionLog,
    CommentCreated,
    Connected,
    Resync,
    Heartbeat,
    #[serde(other)]
    Unknown,
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
            Self::SessionCreated => "session_created",
            Self::SessionUpdated => "session_updated",
            Self::DocumentCreated => "document_created",
            Self::DocumentUpdated => "document_updated",
            Self::DocumentDeleted => "document_deleted",
            Self::LabelCreated => "label_created",
            Self::LabelUpdated => "label_updated",
            Self::LabelDeleted => "label_deleted",
            Self::ConversationCreated => "conversation_created",
            Self::ConversationUpdated => "conversation_updated",
            Self::SessionEventCreated => "session_event_created",
            Self::SessionStateUpdated => "session_state_updated",
            Self::SessionLog => "session_log",
            Self::CommentCreated => "comment_created",
            Self::Connected => "connected",
            Self::Resync => "resync",
            Self::Heartbeat => "heartbeat",
            Self::Unknown => "unknown",
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
            "session_created" | "job_created" => Ok(Self::SessionCreated),
            "session_updated" | "job_updated" => Ok(Self::SessionUpdated),
            "document_created" => Ok(Self::DocumentCreated),
            "document_updated" => Ok(Self::DocumentUpdated),
            "document_deleted" => Ok(Self::DocumentDeleted),
            "label_created" => Ok(Self::LabelCreated),
            "label_updated" => Ok(Self::LabelUpdated),
            "label_deleted" => Ok(Self::LabelDeleted),
            "conversation_created" => Ok(Self::ConversationCreated),
            "conversation_updated" => Ok(Self::ConversationUpdated),
            "session_event_created" => Ok(Self::SessionEventCreated),
            "session_state_updated" => Ok(Self::SessionStateUpdated),
            "session_log" => Ok(Self::SessionLog),
            "comment_created" => Ok(Self::CommentCreated),
            "connected" => Ok(Self::Connected),
            "snapshot" => Ok(Self::Connected),
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
    /// (e.g., `IssueVersionRecord`, `SessionVersionRecord`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<serde_json::Value>,
}

/// Data payload for the connected event sent on initial SSE connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ConnectedEventData {
    /// The current event sequence number for reconnection support.
    pub current_seq: u64,
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

/// Data payload for `session_log` events, emitted on `/v1/events` when the
/// caller has subscribed to one or more `session_ids`. Carries a single log
/// chunk for the named session so consumers can multiplex per-session log
/// streams over the global events SSE rather than opening a separate
/// EventSource per session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SessionLogEventData {
    pub session_id: String,
    pub chunk: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_event_type_unknown_string_deserializes_to_unknown() {
        let event: SseEventType = serde_json::from_str("\"some_future_event\"").unwrap();
        assert_eq!(event, SseEventType::Unknown);
    }

    #[test]
    fn sse_event_type_known_variants_round_trip() {
        let cases = [
            (SseEventType::IssueCreated, "\"issue_created\""),
            (SseEventType::PatchUpdated, "\"patch_updated\""),
            (SseEventType::SessionLog, "\"session_log\""),
            (SseEventType::CommentCreated, "\"comment_created\""),
            (SseEventType::Connected, "\"connected\""),
            (SseEventType::Heartbeat, "\"heartbeat\""),
        ];
        for (variant, wire) in cases {
            let serialized = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialized, wire);
            let deserialized: SseEventType = serde_json::from_str(wire).unwrap();
            assert_eq!(deserialized, variant);
        }
    }
}
