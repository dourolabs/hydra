use crate::{ConversationId, SessionId, users::Username};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchConversationsQuery {
    /// Free-text search across conversation title, agent name, and ID.
    #[serde(default)]
    pub q: Option<String>,
    /// Filter by conversation status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ConversationStatus>,
    /// Filter by creator username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Active,
    Idle,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConversationEvent {
    UserMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    AssistantMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    Suspending {
        reason: String,
        timestamp: DateTime<Utc>,
    },
    Resumed {
        session_id: SessionId,
        timestamp: DateTime<Utc>,
    },
    Closed {
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Conversation {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<String>,
    pub active_session_id: Option<SessionId>,
    pub status: ConversationStatus,
    pub creator: Username,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<String>,
        active_session_id: Option<SessionId>,
        status: ConversationStatus,
        creator: Username,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            conversation_id,
            title,
            agent_name,
            active_session_id,
            status,
            creator,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ConversationSummary {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<String>,
    pub status: ConversationStatus,
    pub event_count: usize,
    pub last_event_preview: Option<String>,
    pub creator: Username,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConversationSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<String>,
        status: ConversationStatus,
        event_count: usize,
        last_event_preview: Option<String>,
        creator: Username,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            conversation_id,
            title,
            agent_name,
            status,
            event_count,
            last_event_preview,
            creator,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CreateConversationRequest {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerConnect {
    Fresh {
        resume_from_event_index: Option<usize>,
    },
    Reconnecting {
        last_received_event_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct WorkerCatchUp {
    pub events: Vec<ConversationEvent>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_bytes"
    )]
    #[cfg_attr(feature = "ts", ts(type = "number[] | null"))]
    pub session_state: Option<Vec<u8>>,
}

mod optional_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_ref().map(|v| v.as_slice()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<Vec<u8>>::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_event_user_message_round_trip() {
        let event = ConversationEvent::UserMessage {
            content: "Hello, agent!".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConversationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
        assert!(json.contains(r#""type":"user_message""#));
    }

    #[test]
    fn conversation_event_assistant_message_round_trip() {
        let event = ConversationEvent::AssistantMessage {
            content: "Hi there!".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConversationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
        assert!(json.contains(r#""type":"assistant_message""#));
    }

    #[test]
    fn conversation_event_suspending_round_trip() {
        let event = ConversationEvent::Suspending {
            reason: "idle_timeout".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConversationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
        assert!(json.contains(r#""type":"suspending""#));
    }

    #[test]
    fn conversation_event_resumed_round_trip() {
        let event = ConversationEvent::Resumed {
            session_id: SessionId::new(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConversationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
        assert!(json.contains(r#""type":"resumed""#));
    }

    #[test]
    fn conversation_event_closed_round_trip() {
        let event = ConversationEvent::Closed {
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConversationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
        assert!(json.contains(r#""type":"closed""#));
    }

    #[test]
    fn conversation_status_round_trip() {
        for status in [
            ConversationStatus::Active,
            ConversationStatus::Idle,
            ConversationStatus::Closed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: ConversationStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn worker_connect_fresh_round_trip() {
        let msg = WorkerConnect::Fresh {
            resume_from_event_index: Some(5),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerConnect = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn worker_connect_reconnecting_round_trip() {
        let msg = WorkerConnect::Reconnecting {
            last_received_event_index: 10,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerConnect = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn worker_catch_up_round_trip() {
        let catch_up = WorkerCatchUp {
            events: vec![ConversationEvent::UserMessage {
                content: "test".to_string(),
                timestamp: Utc::now(),
            }],
            session_state: Some(vec![1, 2, 3]),
        };
        let json = serde_json::to_string(&catch_up).unwrap();
        let deserialized: WorkerCatchUp = serde_json::from_str(&json).unwrap();
        assert_eq!(catch_up, deserialized);
    }

    #[test]
    fn worker_catch_up_without_session_state() {
        let catch_up = WorkerCatchUp {
            events: vec![],
            session_state: None,
        };
        let json = serde_json::to_string(&catch_up).unwrap();
        assert!(!json.contains("session_state"));
        let deserialized: WorkerCatchUp = serde_json::from_str(&json).unwrap();
        assert_eq!(catch_up, deserialized);
    }

    #[test]
    fn create_conversation_request_without_agent_name() {
        let json = r#"{"message":"Hello"}"#;
        let req: CreateConversationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "Hello");
        assert_eq!(req.agent_name, None);
    }
}
