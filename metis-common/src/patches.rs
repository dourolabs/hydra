use crate::{PatchId, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub contents: String,
    pub is_approved: bool,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub diff: String,
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
}
