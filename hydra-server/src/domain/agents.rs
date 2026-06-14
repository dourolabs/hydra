use super::issues::SessionSettings;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Server-side domain agent type.
///
/// Agents are non-versioned: they are created, updated in-place, and soft-archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub prompt_path: String,
    pub mcp_config_path: Option<String>,
    pub max_tries: i32,
    pub max_simultaneous_interactive: i32,
    pub max_simultaneous_headless: i32,
    pub is_default_conversation_agent: bool,
    pub secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        prompt_path: String,
        mcp_config_path: Option<String>,
        max_tries: i32,
        max_simultaneous_interactive: i32,
        max_simultaneous_headless: i32,
        is_default_conversation_agent: bool,
        secrets: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            name,
            prompt_path,
            mcp_config_path,
            max_tries,
            max_simultaneous_interactive,
            max_simultaneous_headless,
            is_default_conversation_agent,
            secrets,
            session_settings: SessionSettings::default(),
            archived: false,
            created_at: now,
            updated_at: now,
        }
    }
}
