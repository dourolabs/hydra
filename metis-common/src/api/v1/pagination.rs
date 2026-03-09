use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};

const MAX_LIMIT: u32 = 200;

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

/// Returns the effective limit capped at MAX_LIMIT, or None if unlimited.
pub fn effective_limit(limit: Option<u32>) -> Option<u32> {
    limit.map(|l| l.min(MAX_LIMIT))
}

/// Computes the `next_cursor` for paginated results using the limit+1 pattern.
///
/// If the result set contains more items than the effective limit, the extra
/// item is removed and a cursor pointing to the last kept item is returned.
pub fn compute_next_cursor<T>(
    records: &mut Vec<T>,
    eff_limit: Option<u32>,
    get_timestamp: impl Fn(&T) -> &DateTime<Utc>,
    get_id: impl Fn(&T) -> &str,
) -> Option<String> {
    let limit = eff_limit?;
    if records.len() > limit as usize {
        records.truncate(limit as usize);
        records
            .last()
            .map(|last| encode_cursor(get_timestamp(last), get_id(last)))
    } else {
        None
    }
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
        assert_eq!(effective_limit(Some(500)), Some(200));
    }

    #[test]
    fn effective_limit_preserves_small_values() {
        assert_eq!(effective_limit(Some(10)), Some(10));
    }

    #[test]
    fn effective_limit_none_when_not_set() {
        assert_eq!(effective_limit(None), None);
    }
}
