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
