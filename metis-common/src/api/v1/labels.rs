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
    #[serde(default = "default_true")]
    pub recurse: bool,
    #[serde(default)]
    pub hidden: bool,
}

fn default_true() -> bool {
    true
}

impl Label {
    pub fn new(name: String, color: Option<Rgb>) -> Self {
        Self {
            name,
            color,
            recurse: true,
            hidden: false,
        }
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
    #[serde(default = "default_true")]
    pub recurse: bool,
    #[serde(default)]
    pub hidden: bool,
}

impl LabelSummary {
    pub fn new(label_id: LabelId, name: String, color: Rgb, recurse: bool, hidden: bool) -> Self {
        Self {
            label_id,
            name,
            color,
            recurse,
            hidden,
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
    #[serde(default = "default_true")]
    pub recurse: bool,
    #[serde(default)]
    pub hidden: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LabelRecord {
    pub fn new(
        label_id: LabelId,
        name: String,
        color: Rgb,
        recurse: bool,
        hidden: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            label_id,
            name,
            color,
            recurse,
            hidden,
            created_at,
            updated_at,
        }
    }
}

/// Request body for creating or updating a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertLabelRequest {
    pub label: Label,
}

impl UpsertLabelRequest {
    pub fn new(label: Label) -> Self {
        Self { label }
    }
}

/// Response body after creating or updating a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertLabelResponse {
    pub label_id: LabelId,
}

impl UpsertLabelResponse {
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
