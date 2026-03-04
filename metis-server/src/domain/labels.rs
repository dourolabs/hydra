use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Server-side domain label type.
///
/// Labels are non-versioned: they are created, updated in-place, and soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Label {
    pub fn new(name: String, color: String) -> Self {
        let now = Utc::now();
        Self {
            name,
            color,
            deleted: false,
            created_at: now,
            updated_at: now,
        }
    }
}
