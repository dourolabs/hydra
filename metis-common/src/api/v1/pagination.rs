use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const MAX_LIMIT: u32 = 200;

/// Pagination parameters accepted by list endpoints.
///
/// When `limit` is provided, cursor-based keyset pagination is active.
/// When `limit` is omitted, all results are returned (backward compatibility).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PaginationParams {
    /// Maximum number of results to return (max 200).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
    /// If true, include `total_count` in the response.
    #[serde(default)]
    pub count: Option<bool>,
}

impl PaginationParams {
    /// Returns the effective limit capped at MAX_LIMIT, or None if unlimited.
    pub fn effective_limit(&self) -> Option<u32> {
        self.limit.map(|l| l.min(MAX_LIMIT))
    }

    /// Returns true if count was requested.
    pub fn wants_count(&self) -> bool {
        self.count.unwrap_or(false)
    }
}

/// Decoded cursor containing the keyset pagination position.
#[derive(Debug, Clone)]
pub struct DecodedCursor {
    pub timestamp: DateTime<Utc>,
    pub id: String,
}

/// Encodes a `(timestamp, id)` cursor as a base64 opaque string.
pub fn encode_cursor(timestamp: &DateTime<Utc>, id: &str) -> String {
    let millis = timestamp.timestamp_millis();
    let raw = format!("{millis}:{id}");
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

/// Decodes a base64 cursor string into `(timestamp, id)`.
pub fn decode_cursor(cursor: &str) -> Result<DecodedCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|e| format!("invalid cursor encoding: {e}"))?;
    let raw = String::from_utf8(bytes).map_err(|e| format!("invalid cursor encoding: {e}"))?;
    let (millis_str, id) = raw
        .split_once(':')
        .ok_or_else(|| "invalid cursor format".to_string())?;
    let millis: i64 = millis_str
        .parse()
        .map_err(|e| format!("invalid cursor timestamp: {e}"))?;
    let timestamp = DateTime::from_timestamp_millis(millis)
        .ok_or_else(|| "invalid cursor timestamp".to_string())?;
    Ok(DecodedCursor {
        timestamp,
        id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trip() {
        let ts = Utc::now();
        let id = "i-abcdefghij";
        let encoded = encode_cursor(&ts, id);
        let decoded = decode_cursor(&encoded).unwrap();
        assert_eq!(decoded.timestamp.timestamp_millis(), ts.timestamp_millis());
        assert_eq!(decoded.id, id);
    }

    #[test]
    fn decode_cursor_rejects_invalid_input() {
        assert!(decode_cursor("not-valid-base64!!!").is_err());
        // Valid base64 but wrong format (no colon)
        let no_colon = URL_SAFE_NO_PAD.encode(b"12345");
        assert!(decode_cursor(&no_colon).is_err());
    }

    #[test]
    fn effective_limit_caps_at_max() {
        let params = PaginationParams {
            limit: Some(500),
            cursor: None,
            count: None,
        };
        assert_eq!(params.effective_limit(), Some(200));
    }

    #[test]
    fn effective_limit_preserves_small_values() {
        let params = PaginationParams {
            limit: Some(10),
            cursor: None,
            count: None,
        };
        assert_eq!(params.effective_limit(), Some(10));
    }

    #[test]
    fn effective_limit_none_when_not_set() {
        let params = PaginationParams::default();
        assert_eq!(params.effective_limit(), None);
    }
}
