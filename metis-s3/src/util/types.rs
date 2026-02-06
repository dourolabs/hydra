use serde::Deserialize;
use std::time::SystemTime;

/// Default maximum number of keys to return in a list operation.
pub const DEFAULT_MAX_KEYS: usize = 1000;

/// Represents a parsed byte range from an HTTP Range header.
/// Supports three forms: bytes=start-end, bytes=start-, bytes=-suffix
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64, // inclusive
}

impl ByteRange {
    /// Resolves the range against the total content length.
    /// Returns None if the range is invalid or unsatisfiable.
    pub fn resolve(range_spec: &str, total_len: u64) -> Option<Self> {
        if total_len == 0 {
            return None;
        }

        let range_spec = range_spec.trim();
        if !range_spec.starts_with("bytes=") {
            return None;
        }

        let range_part = &range_spec[6..];

        // S3 doesn't support multiple ranges
        if range_part.contains(',') {
            return None;
        }

        let parts: Vec<&str> = range_part.split('-').collect();
        if parts.len() != 2 {
            return None;
        }

        let start_str = parts[0].trim();
        let end_str = parts[1].trim();

        if start_str.is_empty() && end_str.is_empty() {
            return None;
        }

        if start_str.is_empty() {
            // Suffix range: bytes=-500 means last 500 bytes
            let suffix_len: u64 = end_str.parse().ok()?;
            if suffix_len == 0 {
                return None;
            }
            let start = total_len.saturating_sub(suffix_len);
            Some(ByteRange {
                start,
                end: total_len - 1,
            })
        } else if end_str.is_empty() {
            // Open-ended range: bytes=500-
            let start: u64 = start_str.parse().ok()?;
            if start >= total_len {
                return None;
            }
            Some(ByteRange {
                start,
                end: total_len - 1,
            })
        } else {
            // Explicit range: bytes=0-999
            let start: u64 = start_str.parse().ok()?;
            let end: u64 = end_str.parse().ok()?;
            if start > end {
                return None;
            }
            if start >= total_len {
                return None;
            }
            // Clamp end to content length - 1
            let end = end.min(total_len - 1);
            Some(ByteRange { start, end })
        }
    }
}

#[derive(Debug)]
pub struct ObjectEntry {
    pub key: String,
    pub last_modified: Option<SystemTime>,
    pub size: u64,
    pub etag: Option<String>,
}

impl PartialEq for ObjectEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for ObjectEntry {}

impl PartialOrd for ObjectEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ObjectEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

#[derive(Debug)]
pub struct ListResult {
    pub entries: Vec<ObjectEntry>,
    pub is_truncated: bool,
    pub next_token: Option<String>,
}

/// Metadata stored for each multipart upload
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MultipartUploadMetadata {
    pub bucket: String,
    pub key: String,
    pub upload_id: String,
    pub created_at: String,
}

/// Query parameters for multipart upload operations
#[derive(Debug, Deserialize)]
pub struct MultipartQuery {
    pub uploads: Option<String>,
    #[serde(rename = "uploadId")]
    pub upload_id: Option<String>,
    #[serde(rename = "partNumber")]
    pub part_number: Option<u32>,
}

/// Part information in CompleteMultipartUpload request body
#[derive(Debug)]
pub struct PartInfo {
    pub part_number: u32,
    pub etag: String,
}

#[derive(Debug, Deserialize)]
pub struct ListObjectsQuery {
    #[serde(rename = "list-type")]
    pub list_type: Option<u8>,
    pub prefix: Option<String>,
    #[serde(rename = "continuation-token")]
    pub continuation_token: Option<String>,
    #[serde(rename = "max-keys")]
    pub max_keys: Option<usize>,
    #[serde(rename = "start-after")]
    pub start_after: Option<String>,
}
