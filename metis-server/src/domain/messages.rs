use super::actors::ActorId;
use serde::{Deserialize, Serialize};

/// The server-side domain message type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

// Conversions between domain and API wire types.
use metis_common::api::v1 as api;

impl From<api::messages::Message> for Message {
    fn from(value: api::messages::Message) -> Self {
        Self {
            sender: value.sender,
            recipient: value.recipient,
            body: value.body,
            deleted: value.deleted,
        }
    }
}

impl From<Message> for api::messages::Message {
    fn from(value: Message) -> Self {
        let mut msg = api::messages::Message::new(value.sender, value.recipient, value.body);
        msg.deleted = value.deleted;
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::users::Username;
    use metis_common::IssueId;

    #[test]
    fn message_domain_roundtrip() {
        let msg = Message::new(
            Some(ActorId::Username(Username::from("alice").into())),
            ActorId::Issue("i-abcdef".parse::<IssueId>().unwrap()),
            "hello".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
        assert!(!decoded.deleted);
    }

    #[test]
    fn message_with_none_sender() {
        let msg = Message::new(
            None,
            ActorId::Issue("i-abcdef".parse::<IssueId>().unwrap()),
            "system notification".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
        assert!(decoded.sender.is_none());
    }

    #[test]
    fn message_api_domain_roundtrip() {
        let api_msg = api::messages::Message::new(
            Some(ActorId::Username(Username::from("alice").into())),
            ActorId::Issue("i-abcdef".parse::<IssueId>().unwrap()),
            "hello".to_string(),
        );

        let domain_msg: Message = api_msg.clone().into();
        let back: api::messages::Message = domain_msg.into();

        assert_eq!(back, api_msg);
    }
}
