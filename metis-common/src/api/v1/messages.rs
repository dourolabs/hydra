use crate::actor_ref::ActorId;
use crate::ids::MessageId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A message exchanged between two actors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Message {
    pub message_id: MessageId,
    pub conversation_id: String,
    pub sender: ActorId,
    pub body: String,
    pub timestamp: DateTime<Utc>,
}

/// Request body for sending a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SendMessageRequest {
    pub recipient: ActorId,
    pub body: String,
}

/// Query parameters for listing messages.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListMessagesQuery {
    /// Filter by conversation partner.
    #[serde(default)]
    pub participant: Option<String>,
    /// Cursor for pagination (message_id). Returns messages before this ID.
    #[serde(default)]
    pub before: Option<String>,
    /// Max messages to return (default: 50).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Response for sending a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SendMessageResponse {
    pub message: Message,
}

/// Response for listing messages (ordered most-recent-first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListMessagesResponse {
    pub messages: Vec<Message>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::users::Username;
    use std::str::FromStr;

    fn sample_message() -> Message {
        Message {
            message_id: MessageId::from_str("m-abcdef").unwrap(),
            conversation_id: "a-i-abcdef+u-alice".to_string(),
            sender: ActorId::Username(Username::from("alice")),
            body: "hello world".to_string(),
            timestamp: chrono::Utc::now(),
        }
    }

    #[test]
    fn message_serialization_round_trip() {
        let msg = sample_message();
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn send_message_request_serialization_round_trip() {
        let req = SendMessageRequest {
            recipient: ActorId::Issue(crate::IssueId::from_str("i-abcdef").unwrap()),
            body: "hello agent".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: SendMessageRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn send_message_response_serialization_round_trip() {
        let resp = SendMessageResponse {
            message: sample_message(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: SendMessageResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, deserialized);
    }

    #[test]
    fn list_messages_response_serialization_round_trip() {
        let resp = ListMessagesResponse {
            messages: vec![sample_message()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ListMessagesResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, deserialized);
    }

    #[test]
    fn list_messages_query_default_has_no_filters() {
        let query = ListMessagesQuery::default();
        assert!(query.participant.is_none());
        assert!(query.before.is_none());
        assert!(query.limit.is_none());
    }
}
