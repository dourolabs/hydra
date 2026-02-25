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
    pub sender: Option<ActorId>,
    pub recipient: ActorId,
    pub body: String,
    #[serde(default)]
    pub deleted: bool,
}

impl Message {
    pub fn new(sender: Option<ActorId>, recipient: ActorId, body: String) -> Self {
        Self {
            sender,
            recipient,
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

/// Query parameters for searching messages.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchMessagesQuery {
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Query parameters for long-poll waiting for new messages.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WaitMessagesQuery {
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub timeout: Option<u32>,
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
            Some(ActorId::Username(Username::from("alice"))),
            ActorId::Issue(crate::IssueId::new()),
            "hello world".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn message_with_none_sender_serde_round_trip() {
        let msg = Message::new(
            None,
            ActorId::Issue(crate::IssueId::new()),
            "system notification".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
        assert!(decoded.sender.is_none());
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
            Some(ActorId::Username(Username::from("alice"))),
            ActorId::Issue(crate::IssueId::new()),
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
            Some(ActorId::Username(Username::from("alice"))),
            ActorId::Issue(crate::IssueId::new()),
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
    fn search_messages_query_defaults() {
        let query = SearchMessagesQuery::default();
        assert_eq!(query.sender, None);
        assert_eq!(query.recipient, None);
        assert_eq!(query.after, None);
        assert_eq!(query.include_deleted, None);
        assert_eq!(query.limit, None);
    }

    #[test]
    fn message_deleted_defaults_to_false() {
        let json = r#"{"sender":{"Username":"alice"},"recipient":{"Username":"bob"},"body":"hi"}"#;
        let msg: Message = serde_json::from_str(json).expect("deserialize");
        assert!(!msg.deleted);
    }
}
