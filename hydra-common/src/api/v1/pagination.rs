use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};

pub const MAX_LIMIT: u32 = 200;

/// The keyset position carried by an opaque cursor.
///
/// Wire format for [`DecodedCursor::encode`] / [`DecodedCursor::decode`]:
/// - `CreatedAtId`: legacy untagged `<micros>:<id>`, kept stable so old
///   cursors keep decoding after newer variants land.
/// - `ProjectStatusTime`: `v2:project_status_time:<priority>:<position>:<micros>:<id>`.
///
/// New variants must introduce a new `vN:<name>:` prefix; never widen the
/// untagged shape, or you re-cut every cursor in flight.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorKeys {
    /// `(created_at DESC, id DESC)` — the default issue-list / patches /
    /// sessions / etc. sort.
    CreatedAtId {
        timestamp: DateTime<Utc>,
        id: String,
    },
    /// `(project.priority ASC, status.position ASC, created_at DESC, id DESC)`
    /// — issue-list grouped by project then status.
    ProjectStatusTime {
        project_priority: f64,
        status_position: f64,
        timestamp: DateTime<Utc>,
        id: String,
    },
}

/// Decoded cursor containing the keyset pagination position.
#[derive(Debug, Clone)]
pub struct DecodedCursor {
    pub keys: CursorKeys,
}

const PROJECT_STATUS_TIME_PREFIX: &str = "v2:project_status_time:";

impl DecodedCursor {
    pub fn created_at_id(timestamp: DateTime<Utc>, id: impl Into<String>) -> Self {
        Self {
            keys: CursorKeys::CreatedAtId {
                timestamp,
                id: id.into(),
            },
        }
    }

    pub fn project_status_time(
        project_priority: f64,
        status_position: f64,
        timestamp: DateTime<Utc>,
        id: impl Into<String>,
    ) -> Self {
        Self {
            keys: CursorKeys::ProjectStatusTime {
                project_priority,
                status_position,
                timestamp,
                id: id.into(),
            },
        }
    }

    /// Encodes this cursor as a base64 opaque string.
    pub fn encode(&self) -> String {
        let raw = match &self.keys {
            CursorKeys::CreatedAtId { timestamp, id } => {
                format!("{}:{id}", timestamp.timestamp_micros())
            }
            CursorKeys::ProjectStatusTime {
                project_priority,
                status_position,
                timestamp,
                id,
            } => format!(
                "{PROJECT_STATUS_TIME_PREFIX}{project_priority}:{status_position}:{}:{id}",
                timestamp.timestamp_micros()
            ),
        };
        URL_SAFE_NO_PAD.encode(raw.as_bytes())
    }

    /// Decodes a base64 cursor string into a `DecodedCursor`.
    pub fn decode(cursor: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|e| format!("invalid cursor encoding: {e}"))?;
        let raw = String::from_utf8(bytes).map_err(|e| format!("invalid cursor encoding: {e}"))?;

        if let Some(payload) = raw.strip_prefix(PROJECT_STATUS_TIME_PREFIX) {
            let mut parts = payload.splitn(4, ':');
            let priority_str = parts.next().ok_or("invalid cursor format")?;
            let position_str = parts.next().ok_or("invalid cursor format")?;
            let micros_str = parts.next().ok_or("invalid cursor format")?;
            let id = parts.next().ok_or("invalid cursor format")?;
            let project_priority: f64 = priority_str
                .parse()
                .map_err(|e| format!("invalid cursor priority: {e}"))?;
            let status_position: f64 = position_str
                .parse()
                .map_err(|e| format!("invalid cursor position: {e}"))?;
            let micros: i64 = micros_str
                .parse()
                .map_err(|e| format!("invalid cursor timestamp: {e}"))?;
            let timestamp = DateTime::from_timestamp_micros(micros)
                .ok_or_else(|| "invalid cursor timestamp".to_string())?;
            return Ok(DecodedCursor {
                keys: CursorKeys::ProjectStatusTime {
                    project_priority,
                    status_position,
                    timestamp,
                    id: id.to_string(),
                },
            });
        }

        let (micros_str, id) = raw
            .split_once(':')
            .ok_or_else(|| "invalid cursor format".to_string())?;
        let micros: i64 = micros_str
            .parse()
            .map_err(|e| format!("invalid cursor timestamp: {e}"))?;
        let timestamp = DateTime::from_timestamp_micros(micros)
            .ok_or_else(|| "invalid cursor timestamp".to_string())?;
        Ok(DecodedCursor {
            keys: CursorKeys::CreatedAtId {
                timestamp,
                id: id.to_string(),
            },
        })
    }
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
    compute_next_cursor_with_keys(records, eff_limit, |r| CursorKeys::CreatedAtId {
        timestamp: *get_timestamp(r),
        id: get_id(r).to_string(),
    })
}

/// Sort-aware variant of [`compute_next_cursor`]: callers supply the
/// [`CursorKeys`] for the cursor row. Use this when paginating against a
/// non-default sort whose cursor carries more than `(timestamp, id)`.
pub fn compute_next_cursor_with_keys<T>(
    records: &mut Vec<T>,
    eff_limit: Option<u32>,
    get_keys: impl Fn(&T) -> CursorKeys,
) -> Option<String> {
    let limit = eff_limit?;
    if records.len() > limit as usize {
        records.truncate(limit as usize);
        records.last().map(|last| {
            let cursor = DecodedCursor {
                keys: get_keys(last),
            };
            cursor.encode()
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_at_id_cursor_round_trips() {
        let ts = Utc::now();
        let id = "i-abcdefghij";
        let cursor = DecodedCursor::created_at_id(ts, id);
        let encoded = cursor.encode();
        let decoded = DecodedCursor::decode(&encoded).unwrap();
        match decoded.keys {
            CursorKeys::CreatedAtId {
                timestamp,
                id: decoded_id,
            } => {
                assert_eq!(timestamp.timestamp_micros(), ts.timestamp_micros());
                assert_eq!(decoded_id, id);
            }
            other => panic!("expected CreatedAtId, got {other:?}"),
        }
    }

    #[test]
    fn project_status_time_cursor_round_trips() {
        let ts = Utc::now();
        let cursor = DecodedCursor::project_status_time(1500.0, 200.0, ts, "i-abcdef");
        let encoded = cursor.encode();
        let decoded = DecodedCursor::decode(&encoded).unwrap();
        match decoded.keys {
            CursorKeys::ProjectStatusTime {
                project_priority,
                status_position,
                timestamp,
                id,
            } => {
                assert_eq!(project_priority, 1500.0);
                assert_eq!(status_position, 200.0);
                assert_eq!(timestamp.timestamp_micros(), ts.timestamp_micros());
                assert_eq!(id, "i-abcdef");
            }
            other => panic!("expected ProjectStatusTime, got {other:?}"),
        }
    }

    /// Pre-PR cursors were emitted as the untagged `<micros>:<id>` shape.
    /// Decoding the legacy shape MUST still produce `CursorKeys::CreatedAtId`
    /// after the v2 variant was added — bumping the format would invalidate
    /// every in-flight client cursor.
    #[test]
    fn legacy_untagged_cursor_still_decodes_as_created_at_id() {
        let ts = Utc::now();
        let micros = ts.timestamp_micros();
        let raw = format!("{micros}:i-legacy");
        let encoded = URL_SAFE_NO_PAD.encode(raw.as_bytes());
        let decoded = DecodedCursor::decode(&encoded).unwrap();
        match decoded.keys {
            CursorKeys::CreatedAtId {
                timestamp,
                id: decoded_id,
            } => {
                assert_eq!(timestamp.timestamp_micros(), micros);
                assert_eq!(decoded_id, "i-legacy");
            }
            other => panic!("expected CreatedAtId, got {other:?}"),
        }
    }

    /// A cursor produced by `sort=project_status_time` MUST NOT decode as
    /// `CreatedAtId` when re-sent (degrading would silently drop pagination
    /// keys and break keyset ordering across pages).
    #[test]
    fn project_status_time_cursor_does_not_degrade_to_created_at_id() {
        let cursor = DecodedCursor::project_status_time(1.0, 2.0, Utc::now(), "i-x");
        let encoded = cursor.encode();
        let decoded = DecodedCursor::decode(&encoded).unwrap();
        assert!(matches!(decoded.keys, CursorKeys::ProjectStatusTime { .. }));
    }

    #[test]
    fn decode_cursor_rejects_invalid_input() {
        assert!(DecodedCursor::decode("not-valid-base64!!!").is_err());
        // Valid base64 but wrong format (no colon)
        let no_colon = URL_SAFE_NO_PAD.encode(b"12345");
        assert!(DecodedCursor::decode(&no_colon).is_err());
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
