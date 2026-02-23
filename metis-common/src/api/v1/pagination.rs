use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default number of items per page.
pub const DEFAULT_PAGE_LIMIT: u32 = 50;
/// Maximum number of items per page.
pub const MAX_PAGE_LIMIT: u32 = 200;

/// Sort direction for paginated results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

impl std::fmt::Display for SortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortOrder::Asc => write!(f, "asc"),
            SortOrder::Desc => write!(f, "desc"),
        }
    }
}

impl std::str::FromStr for SortOrder {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "asc" => Ok(SortOrder::Asc),
            "desc" => Ok(SortOrder::Desc),
            other => Err(format!(
                "invalid sort order: '{other}', expected 'asc' or 'desc'"
            )),
        }
    }
}

/// Parameters for cursor-based pagination, extracted from query parameters.
#[derive(Debug, Clone, Default)]
pub struct PaginationParams {
    /// Maximum items per page. Clamped to [1, MAX_PAGE_LIMIT].
    pub limit: u32,
    /// Opaque cursor from previous response. None = first page.
    pub cursor: Option<CursorData>,
    /// Sort direction by timestamp.
    pub sort: SortOrder,
}

impl PaginationParams {
    pub fn new(limit: Option<u32>, cursor: Option<String>, sort: Option<SortOrder>) -> Self {
        let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT);
        let cursor = cursor.and_then(|c| CursorData::decode(&c).ok());
        let sort = sort.unwrap_or_default();
        Self {
            limit,
            cursor,
            sort,
        }
    }
}

/// Decoded cursor containing the last item's (timestamp, id) for keyset pagination.
#[derive(Debug, Clone)]
pub struct CursorData {
    pub timestamp: DateTime<Utc>,
    pub id: String,
}

impl CursorData {
    pub fn new(timestamp: DateTime<Utc>, id: String) -> Self {
        Self { timestamp, id }
    }

    /// Encode cursor data to an opaque base64 string.
    pub fn encode(&self) -> String {
        let payload = format!("{}|{}", self.timestamp.to_rfc3339(), self.id);
        URL_SAFE_NO_PAD.encode(payload.as_bytes())
    }

    /// Decode a cursor string back to CursorData.
    pub fn decode(cursor: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|e| format!("invalid cursor encoding: {e}"))?;
        let payload = String::from_utf8(bytes).map_err(|e| format!("invalid cursor utf8: {e}"))?;
        let (ts_str, id) = payload
            .split_once('|')
            .ok_or_else(|| "invalid cursor format".to_string())?;
        let timestamp = DateTime::parse_from_rfc3339(ts_str)
            .map_err(|e| format!("invalid cursor timestamp: {e}"))?
            .with_timezone(&Utc);
        Ok(Self {
            timestamp,
            id: id.to_string(),
        })
    }
}

/// Paginated response wrapper for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PaginatedResponse<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Opaque cursor for the next page. None when no more results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Total matching items (for UI "showing X of Y").
    pub total_count: u32,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, next_cursor: Option<String>, total_count: u32) -> Self {
        Self {
            items,
            next_cursor,
            total_count,
        }
    }
}
