use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::LabelId;
use crate::rgb::Rgb;

/// The input representation of a label (name + optional color).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Label {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgb>,
}

impl Label {
    pub fn new(name: String, color: Option<Rgb>) -> Self {
        Self { name, color }
    }
}

/// A lightweight label representation for embedding in other responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct LabelSummary {
    pub label_id: LabelId,
    pub name: String,
    pub color: Rgb,
}

impl LabelSummary {
    pub fn new(label_id: LabelId, name: String, color: Rgb) -> Self {
        Self {
            label_id,
            name,
            color,
        }
    }
}

/// Full label record returned by GET endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct LabelRecord {
    pub label_id: LabelId,
    pub name: String,
    pub color: Rgb,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LabelRecord {
    pub fn new(
        label_id: LabelId,
        name: String,
        color: Rgb,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            label_id,
            name,
            color,
            created_at,
            updated_at,
        }
    }
}

/// Request body for creating a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateLabelRequest {
    pub label: Label,
}

/// Response body after creating a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateLabelResponse {
    pub label_id: LabelId,
}

impl CreateLabelResponse {
    pub fn new(label_id: LabelId) -> Self {
        Self { label_id }
    }
}

/// Query parameters for listing labels.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchLabelsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

/// Response body for listing labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListLabelsResponse {
    pub labels: Vec<LabelRecord>,
}

impl ListLabelsResponse {
    pub fn new(labels: Vec<LabelRecord>) -> Self {
        Self { labels }
    }
}
