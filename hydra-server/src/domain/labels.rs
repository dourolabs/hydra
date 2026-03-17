use chrono::{DateTime, Utc};
use hydra_common::Rgb;
use serde::{Deserialize, Serialize};

/// Server-side domain label type.
///
/// Labels are non-versioned: they are created, updated in-place, and soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: Rgb,
    pub deleted: bool,
    pub recurse: bool,
    pub hidden: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Label {
    pub fn new(name: String, color: Rgb, recurse: bool, hidden: bool) -> Self {
        let now = Utc::now();
        Self {
            name,
            color,
            deleted: false,
            recurse,
            hidden,
            created_at: now,
            updated_at: now,
        }
    }
}
