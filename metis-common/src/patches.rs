use crate::{PatchId, TaskId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchStatus {
    Open,
    Closed,
    Merged,
}

impl Default for PatchStatus {
    fn default() -> Self {
        Self::Open
    }
}

impl PatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PatchStatus::Open => "open",
            PatchStatus::Closed => "closed",
            PatchStatus::Merged => "merged",
        }
    }
}

impl fmt::Display for PatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PatchStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "open" => Ok(PatchStatus::Open),
            "closed" => Ok(PatchStatus::Closed),
            "merged" => Ok(PatchStatus::Merged),
            other => Err(format!("unsupported patch status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub contents: String,
    pub is_approved: bool,
    pub author: String,
    /// Timestamp for when the review was recorded.
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub diff: String,
    #[serde(default)]
    pub status: PatchStatus,
    /// True when the patch is an automatic backup created from a job's output after tool-use patch generation failed.
    #[serde(default)]
    pub is_automatic_backup: bool,
    #[serde(default)]
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchRecord {
    pub id: PatchId,
    pub patch: Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchRequest {
    pub patch: Patch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchResponse {
    pub patch_id: PatchId,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPatchesQuery {
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPatchesResponse {
    pub patches: Vec<PatchRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_patches_query_serializes_with_reqwest() {
        let query = SearchPatchesQuery {
            q: Some("test query".to_string()),
        };

        // Test that reqwest can serialize the query when building the request
        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/patches")
            .query(&query)
            .build();
        result.expect("Failed to serialize SearchPatchesQuery with reqwest");
    }

    #[test]
    fn search_patches_query_serializes_empty_query() {
        let query = SearchPatchesQuery::default();

        // Test that reqwest can serialize an empty query when building the request
        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/patches")
            .query(&query)
            .build();
        result.expect("Failed to serialize empty SearchPatchesQuery");
    }

    #[test]
    fn patch_status_parses_from_strings() {
        assert_eq!(PatchStatus::from_str("open").unwrap(), PatchStatus::Open);
        assert_eq!(
            PatchStatus::from_str("CLOSED").unwrap(),
            PatchStatus::Closed
        );
        assert_eq!(
            PatchStatus::from_str(" merged ").unwrap(),
            PatchStatus::Merged
        );
        assert!(PatchStatus::from_str("pending").is_err());
    }
}
