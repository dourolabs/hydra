use crate::actor_ref::{ActorId, ActorRef};
use crate::{MessageId, VersionNumber};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The message domain type (inner type for Versioned<Message>).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Message {
    pub conversation_id: String,
    pub sender: ActorId,
    pub body: String,
    #[serde(default)]
    pub deleted: bool,
}

impl Message {
    pub fn new(conversation_id: String, sender: ActorId, body: String) -> Self {
        Self {
            conversation_id,
            sender,
            body,
            deleted: false,
        }
    }
}

/// Flattened representation of Versioned<Message> for the wire.
/// Follows the same pattern as other versioned entities in the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct VersionedMessage {
    pub message_id: MessageId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
}

impl VersionedMessage {
    pub fn new(
        message_id: MessageId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        message: Message,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
    ) -> Self {
        Self {
            message_id,
            version,
            timestamp,
            message,
            actor,
            creation_time,
        }
    }
}

/// Request to send a new message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SendMessageRequest {
    pub recipient: ActorId,
    pub body: String,
}

impl SendMessageRequest {
    pub fn new(recipient: ActorId, body: String) -> Self {
        Self { recipient, body }
    }
}

/// Query parameters for listing messages.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListMessagesQuery {
    #[serde(default)]
    pub participant: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Response after sending a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SendMessageResponse {
    pub message_id: MessageId,
    pub version: VersionNumber,
    pub message: Message,
    pub timestamp: DateTime<Utc>,
}

impl SendMessageResponse {
    pub fn new(
        message_id: MessageId,
        version: VersionNumber,
        message: Message,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            message_id,
            version,
            message,
            timestamp,
        }
    }
}

/// Response containing a list of messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListMessagesResponse {
    pub messages: Vec<VersionedMessage>,
}

impl ListMessagesResponse {
    pub fn new(messages: Vec<VersionedMessage>) -> Self {
        Self { messages }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::users::Username;

    #[test]
    fn message_serde_round_trip() {
        let msg = Message::new(
            "a-i-abc+u-alice".to_string(),
            ActorId::Username(Username::from("alice")),
            "hello world".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn send_message_request_serde_round_trip() {
        let req = SendMessageRequest::new(
            ActorId::Issue(crate::IssueId::new()),
            "test body".to_string(),
        );

        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: SendMessageRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, req);
    }

    #[test]
    fn versioned_message_serde_round_trip() {
        let msg = Message::new(
            "a-i-abc+u-alice".to_string(),
            ActorId::Username(Username::from("alice")),
            "hello".to_string(),
        );
        let ts = chrono::Utc::now();
        let vm = VersionedMessage::new(MessageId::new(), 1, ts, msg.clone(), None, ts);

        let json = serde_json::to_string(&vm).expect("serialize");
        let decoded: VersionedMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.message, msg);
        assert_eq!(decoded.version, 1);
    }

    #[test]
    fn versioned_message_omits_actor_when_none() {
        let msg = Message::new(
            "a-i-abc+u-alice".to_string(),
            ActorId::Username(Username::from("alice")),
            "hello".to_string(),
        );
        let ts = chrono::Utc::now();
        let vm = VersionedMessage::new(MessageId::new(), 1, ts, msg, None, ts);

        let value = serde_json::to_value(&vm).expect("serialize");
        assert!(
            value.get("actor").is_none(),
            "actor should be omitted when None"
        );
    }

    #[test]
    fn list_messages_query_defaults() {
        let query = ListMessagesQuery::default();
        assert_eq!(query.participant, None);
        assert_eq!(query.before, None);
        assert_eq!(query.limit, None);
    }

    #[test]
    fn message_deleted_defaults_to_false() {
        let json = r#"{"conversation_id":"a+b","sender":{"Username":"alice"},"body":"hi"}"#;
        let msg: Message = serde_json::from_str(json).expect("deserialize");
        assert!(!msg.deleted);
    }
}
