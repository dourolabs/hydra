use crate::MetisId;
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
    #[serde(default)]
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchRecord {
    pub id: MetisId,
    pub patch: Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchRequest {
    pub patch: Patch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<MetisId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertPatchResponse {
    pub patch_id: MetisId,
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
