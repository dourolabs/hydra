use serde::{Deserialize, Serialize};

/// A single relation in the response.
///
/// Note: intentionally omits `source_kind`, `target_kind`, and `created_at`
/// per the design document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RelationResponse {
    pub source_id: String,
    pub target_id: String,
    pub rel_type: String,
}

/// Query parameters for `GET /v1/relations/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListRelationsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Comma-separated list of source IDs (mutually exclusive with `source_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ids: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Comma-separated list of target IDs (mutually exclusive with `target_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ids: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitive: Option<bool>,
}

/// Response body for `GET /v1/relations/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListRelationsResponse {
    pub relations: Vec<RelationResponse>,
}

/// Request body for `POST /v1/relations/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CreateRelationRequest {
    pub source_id: String,
    pub target_id: String,
    pub rel_type: String,
}

/// Request body for `DELETE /v1/relations/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RemoveRelationRequest {
    pub source_id: String,
    pub target_id: String,
    pub rel_type: String,
}

/// Response body for `DELETE /v1/relations/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RemoveRelationResponse {
    pub removed: bool,
}
