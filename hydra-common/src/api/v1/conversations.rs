use super::agents::AgentName;
use super::issues::SessionSettings;
use crate::{ConversationId, IssueId, users::Username};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Filter by the issue that spawned this conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
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
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ConversationStatus {
    Active,
    Idle,
    Closed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Conversation {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<AgentName>,
    pub status: ConversationStatus,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<AgentName>,
        status: ConversationStatus,
        creator: Username,
        session_settings: SessionSettings,
        spawned_from: Option<IssueId>,
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
            spawned_from,
            created_at,
            updated_at,
        }
    }
}

impl crate::graph::GraphView for Conversation {
    const KIND: crate::graph::ObjectKind = crate::graph::ObjectKind::Conversation;

    fn view_l1(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "status": self.status,
        })
    }

    fn view_l2(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "status": self.status,
            "agent_name": self.agent_name,
            "creator": self.creator,
            "updated_at": self.updated_at,
        })
    }

    fn view_l3(&self) -> Value {
        serde_json::to_value(self).expect("Conversation serialization is infallible")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ConversationSummary {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<AgentName>,
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
        agent_name: Option<AgentName>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<AgentName>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
    fn conversation_status_unknown_string_deserializes_to_unknown() {
        let status: ConversationStatus = serde_json::from_str("\"archived\"").unwrap();
        assert_eq!(status, ConversationStatus::Unknown);
    }

    #[test]
    fn create_conversation_request_without_agent_name() {
        let json = r#"{"message":"Hello"}"#;
        let req: CreateConversationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message.as_deref(), Some("Hello"));
        assert_eq!(req.agent_name, None);
        assert_eq!(req.session_settings, None);
    }

    #[test]
    fn create_conversation_request_with_session_settings_round_trip() {
        let req = CreateConversationRequest {
            message: Some("Hello".to_string()),
            agent_name: Some(AgentName::try_new("my-agent").unwrap()),
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
            message: Some("Hello".to_string()),
            agent_name: None,
            session_settings: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("session_settings"));
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn create_conversation_request_without_message_round_trip() {
        let req = CreateConversationRequest {
            message: None,
            agent_name: None,
            session_settings: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("message"));
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn create_conversation_request_deserializes_empty_object() {
        let req: CreateConversationRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.message, None);
        assert_eq!(req.agent_name, None);
        assert_eq!(req.session_settings, None);
    }

    #[test]
    fn conversation_round_trips_with_spawned_from_some() {
        use chrono::TimeZone;
        let issue_id = IssueId::from_str("i-testid").unwrap();
        let mut conv = Conversation::new(
            ConversationId::new(),
            None,
            None,
            ConversationStatus::Active,
            Username::from("alice"),
            SessionSettings::default(),
            Some(issue_id.clone()),
            Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 5, 2, 12, 0, 0).unwrap(),
        );
        conv.spawned_from = Some(issue_id.clone());
        let json = serde_json::to_string(&conv).unwrap();
        assert!(json.contains("spawned_from"));
        let de: Conversation = serde_json::from_str(&json).unwrap();
        assert_eq!(de.spawned_from, Some(issue_id));
        assert_eq!(de, conv);
    }

    #[test]
    fn conversation_omits_spawned_from_when_none() {
        use chrono::TimeZone;
        let conv = Conversation::new(
            ConversationId::new(),
            None,
            None,
            ConversationStatus::Active,
            Username::from("alice"),
            SessionSettings::default(),
            None,
            Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 5, 2, 12, 0, 0).unwrap(),
        );
        let json = serde_json::to_string(&conv).unwrap();
        assert!(!json.contains("spawned_from"));
        let de: Conversation = serde_json::from_str(&json).unwrap();
        assert_eq!(de.spawned_from, None);
    }

    #[test]
    fn search_conversations_query_round_trips_spawned_from() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let query = SearchConversationsQuery {
            spawned_from: Some(issue_id.clone()),
            ..SearchConversationsQuery::default()
        };
        let json = serde_json::to_string(&query).unwrap();
        assert!(json.contains("spawned_from"));
        let de: SearchConversationsQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(de.spawned_from, Some(issue_id));
    }

    #[test]
    fn search_conversations_query_omits_spawned_from_when_none() {
        let query = SearchConversationsQuery::default();
        let json = serde_json::to_string(&query).unwrap();
        assert!(!json.contains("spawned_from"));
    }

    mod graph_view {
        use super::*;
        use crate::graph::{GraphView, ObjectKind};
        use chrono::TimeZone;
        use serde_json::json;

        fn sample_conversation() -> Conversation {
            Conversation::new(
                ConversationId::new(),
                Some("Triaging flake".to_string()),
                Some(AgentName::try_new("claude").unwrap()),
                ConversationStatus::Active,
                Username::from("alice"),
                SessionSettings::default(),
                None,
                Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 2, 12, 0, 0).unwrap(),
            )
        }

        #[test]
        fn kind_is_conversation() {
            assert_eq!(<Conversation as GraphView>::KIND, ObjectKind::Conversation);
        }

        #[test]
        fn view_l1_matches_expected() {
            let conv = sample_conversation();
            assert_eq!(
                conv.view_l1(),
                json!({
                    "title": "Triaging flake",
                    "status": "active",
                })
            );
        }

        #[test]
        fn view_l2_matches_expected() {
            let conv = sample_conversation();
            assert_eq!(
                conv.view_l2(),
                json!({
                    "title": "Triaging flake",
                    "status": "active",
                    "agent_name": "claude",
                    "creator": "alice",
                    "updated_at": "2026-05-02T12:00:00Z",
                })
            );
        }

        #[test]
        fn view_l2_contains_view_l1_keys_with_same_values() {
            let conv = sample_conversation();
            let l1 = conv.view_l1();
            let l2 = conv.view_l2();
            for (key, expected) in l1.as_object().unwrap() {
                assert_eq!(l2.get(key), Some(expected), "key {key} mismatch in L2");
            }
        }
    }
}
