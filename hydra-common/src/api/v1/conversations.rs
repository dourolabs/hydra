use super::issues::SessionSettings;
use crate::{ConversationId, SessionId, users::Username};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ConversationEventId {
    pub conversation_id: ConversationId,
    pub event_index: usize,
}

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
    /// Include soft-deleted conversations in results.
    #[serde(default)]
    pub include_deleted: Option<bool>,
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
    pub status: ConversationStatus,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<String>,
        status: ConversationStatus,
        creator: Username,
        session_settings: SessionSettings,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            conversation_id,
            title,
            agent_name,
            status,
            creator,
            session_settings,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_settings: Option<SessionSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UpdateConversationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SendMessageRequest {
    pub content: String,
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

/// Messages sent from the worker to the server over the relay WebSocket.
///
/// This enum distinguishes between conversation events (which get stored and
/// broadcast) and session state uploads (binary blobs for resumption).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// A conversation event (user message, assistant message, suspending, etc.).
    Event { event: ConversationEvent },
    /// A session state upload for resumption support.
    SessionStateUpload {
        #[cfg_attr(feature = "ts", ts(type = "number[]"))]
        data: Vec<u8>,
    },
}

/// Messages sent from the server to the worker over the relay WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Catch-up payload sent immediately after the worker connects.
    CatchUp(WorkerCatchUp),
    /// A conversation event forwarded to the worker (e.g., a user message).
    Event { event: ConversationEvent },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
        assert_eq!(req.session_settings, None);
    }

    #[test]
    fn create_conversation_request_with_session_settings_round_trip() {
        let req = CreateConversationRequest {
            message: "Hello".to_string(),
            agent_name: Some("my-agent".to_string()),
            session_settings: Some(SessionSettings {
                repo_name: Some(crate::RepoName::from_str("org/repo").unwrap()),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
        assert!(json.contains("session_settings"));
        assert!(json.contains("org/repo"));
    }

    #[test]
    fn create_conversation_request_without_session_settings_omits_field() {
        let req = CreateConversationRequest {
            message: "Hello".to_string(),
            agent_name: None,
            session_settings: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("session_settings"));
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn worker_message_event_round_trip() {
        let msg = WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "Hello!".to_string(),
                timestamp: Utc::now(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"event""#));
    }

    #[test]
    fn worker_message_session_state_upload_round_trip() {
        let msg = WorkerMessage::SessionStateUpload {
            data: vec![10, 20, 30],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"session_state_upload""#));
    }

    #[test]
    fn server_message_catch_up_round_trip() {
        let msg = ServerMessage::CatchUp(WorkerCatchUp {
            events: vec![ConversationEvent::UserMessage {
                content: "hi".to_string(),
                timestamp: Utc::now(),
            }],
            session_state: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn server_message_event_round_trip() {
        let msg = ServerMessage::Event {
            event: ConversationEvent::UserMessage {
                content: "hello".to_string(),
                timestamp: Utc::now(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }
}
