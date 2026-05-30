use super::issues::SessionSettings;
use super::users::Username;
use hydra_common::ConversationId;
use hydra_common::api::v1 as api;
use hydra_common::api::v1::agents::AgentName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    #[default]
    Active,
    Idle,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub title: Option<String>,
    pub agent_name: Option<AgentName>,
    #[serde(default)]
    pub status: ConversationStatus,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    #[serde(default)]
    pub deleted: bool,
}

// ---- From conversions: API -> Domain ----

impl From<api::conversations::ConversationStatus> for ConversationStatus {
    fn from(value: api::conversations::ConversationStatus) -> Self {
        match value {
            api::conversations::ConversationStatus::Active => ConversationStatus::Active,
            api::conversations::ConversationStatus::Idle => ConversationStatus::Idle,
            api::conversations::ConversationStatus::Closed => ConversationStatus::Closed,
        }
    }
}

impl From<ConversationStatus> for api::conversations::ConversationStatus {
    fn from(value: ConversationStatus) -> Self {
        match value {
            ConversationStatus::Active => api::conversations::ConversationStatus::Active,
            ConversationStatus::Idle => api::conversations::ConversationStatus::Idle,
            ConversationStatus::Closed => api::conversations::ConversationStatus::Closed,
        }
    }
}

impl From<api::conversations::Conversation> for Conversation {
    fn from(value: api::conversations::Conversation) -> Self {
        Self {
            title: value.title,
            agent_name: value.agent_name,
            status: value.status.into(),
            creator: value.creator.into(),
            session_settings: value.session_settings.into(),
            deleted: false,
        }
    }
}

impl Conversation {
    /// Convert to API Conversation type, filling in the ID and timestamps from Versioned metadata.
    pub fn to_api(
        &self,
        conversation_id: ConversationId,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> api::conversations::Conversation {
        api::conversations::Conversation::new(
            conversation_id,
            self.title.clone(),
            self.agent_name.clone(),
            self.status.into(),
            self.creator.clone().into(),
            self.session_settings.clone().into(),
            created_at,
            updated_at,
        )
    }
}
