use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Server-side domain agent type.
///
/// Agents are non-versioned: they are created, updated in-place, and soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub prompt_path: String,
    pub max_tries: i32,
    pub max_simultaneous: i32,
    pub is_assignment_agent: bool,
    pub secrets: Vec<String>,
    pub deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Agent {
    pub fn new(
        name: String,
        prompt_path: String,
        max_tries: i32,
        max_simultaneous: i32,
        is_assignment_agent: bool,
        secrets: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            name,
            prompt_path,
            max_tries,
            max_simultaneous,
            is_assignment_agent,
            secrets,
            deleted: false,
            created_at: now,
            updated_at: now,
        }
    }
}
