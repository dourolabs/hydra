use super::events::{EntityEventData, SseEventType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Query parameters for the GET /v1/activity endpoint.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchActivityQuery {
    /// Maximum number of events to return (1-200). Defaults to 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor for pagination.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Comma-separated entity types to filter (e.g. "issues,patches").
    #[serde(default)]
    pub entity_types: Option<String>,
    /// Filter by actor identifier.
    #[serde(default)]
    pub actor: Option<String>,
    /// Only return events at or after this time (inclusive).
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    /// Only return events before this time (exclusive).
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
}

/// A single item in the activity feed, wrapping an SSE event type with entity event data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ActivityFeedItem {
    pub event_type: SseEventType,
    #[serde(flatten)]
    pub data: EntityEventData,
}

/// Response from the GET /v1/activity endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ActivityFeedResponse {
    /// Events in reverse chronological order (most recent first).
    pub events: Vec<ActivityFeedItem>,
    /// Map of base objects for client-side diff computation.
    /// Keyed by "{entity_type}:{entity_id}:{version}" (the version N-1 record).
    pub base_objects: HashMap<String, serde_json::Value>,
    /// Opaque cursor for fetching the next page, or null if no more results.
    pub next_cursor: Option<String>,
}

/// Determines the [`SseEventType`] for a given entity mutation.
///
/// This centralizes the event-type classification logic shared between store
/// implementations (Postgres and in-memory):
/// - Version 1 ⇒ `*Created`
/// - `deleted` is true and `base_deleted` is false ⇒ `*Deleted` (first deletion)
/// - Otherwise ⇒ `*Updated`
///
/// Returns `None` for unrecognized entity types.
pub fn classify_event_type(
    entity_type: &str,
    version: u64,
    deleted: bool,
    base_deleted: bool,
) -> Option<SseEventType> {
    if version == 1 {
        match entity_type {
            "issue" => Some(SseEventType::IssueCreated),
            "patch" => Some(SseEventType::PatchCreated),
            "job" => Some(SseEventType::JobCreated),
            "document" => Some(SseEventType::DocumentCreated),
            _ => None,
        }
    } else if deleted && !base_deleted {
        match entity_type {
            "issue" => Some(SseEventType::IssueDeleted),
            "patch" => Some(SseEventType::PatchDeleted),
            "document" => Some(SseEventType::DocumentDeleted),
            // Jobs don't have a "deleted" event type
            "job" => Some(SseEventType::JobUpdated),
            _ => None,
        }
    } else {
        match entity_type {
            "issue" => Some(SseEventType::IssueUpdated),
            "patch" => Some(SseEventType::PatchUpdated),
            "job" => Some(SseEventType::JobUpdated),
            "document" => Some(SseEventType::DocumentUpdated),
            _ => None,
        }
    }
}

/// Extracts the `deleted` flag from a serialized version record JSON value.
///
/// The deleted field is nested inside the entity-specific key (e.g., `.issue.deleted`,
/// `.patch.deleted`, `.task.deleted`, or `.document.deleted`).
pub fn extract_deleted_from_json(json: &serde_json::Value) -> bool {
    json.get("issue")
        .or(json.get("patch"))
        .or(json.get("task"))
        .or(json.get("document"))
        .and_then(|entity| entity.get("deleted"))
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
}

/// Internal cursor representation for pagination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityCursor {
    pub ts: DateTime<Utc>,
    pub id: String,
    pub v: u64,
}

impl ActivityCursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("cursor serialization should not fail");
        // Hex-encode the JSON to produce a URL-safe opaque cursor
        json.bytes().map(|b| format!("{b:02x}")).collect()
    }

    pub fn decode(s: &str) -> Option<Self> {
        // Decode hex back to JSON
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}
