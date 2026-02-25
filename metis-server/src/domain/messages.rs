use super::actors::ActorId;
use serde::{Deserialize, Serialize};

/// A canonical conversation identifier built from two actor names.
///
/// The two actor name strings are sorted lexicographically and joined with `+`.
/// For example: `a-i-abcdef+u-alice`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(String);

impl ConversationId {
    /// Build the canonical conversation ID from two actors.
    ///
    /// The two actor names are sorted lexicographically so that the
    /// conversation between A and B is always the same regardless of
    /// who initiates it.
    pub fn from_pair(a: &ActorId, b: &ActorId) -> Self {
        let name_a = a.to_string();
        let name_b = b.to_string();
        let (first, second) = if name_a <= name_b {
            (name_a, name_b)
        } else {
            (name_b, name_a)
        };
        Self(format!("{first}+{second}"))
    }

    /// Returns the canonical conversation ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ConversationId> for String {
    fn from(value: ConversationId) -> Self {
        value.0
    }
}

/// The server-side domain message type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

// Conversions between domain and API wire types.
use metis_common::api::v1 as api;

impl From<api::messages::Message> for Message {
    fn from(value: api::messages::Message) -> Self {
        Self {
            conversation_id: value.conversation_id,
            sender: value.sender,
            body: value.body,
            deleted: value.deleted,
        }
    }
}

impl From<Message> for api::messages::Message {
    fn from(value: Message) -> Self {
        let mut msg = api::messages::Message::new(value.conversation_id, value.sender, value.body);
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
    fn conversation_id_canonical_order() {
        let user = ActorId::Username(Username::from("alice").into());
        let issue: IssueId = "i-abcd".parse().unwrap();
        let agent = ActorId::Issue(issue);

        let id1 = ConversationId::from_pair(&user, &agent);
        let id2 = ConversationId::from_pair(&agent, &user);

        assert_eq!(id1, id2, "order of arguments should not matter");
        // "a-i-abcd" < "u-alice" lexicographically
        assert_eq!(id1.as_str(), "a-i-abcd+u-alice");
    }

    #[test]
    fn conversation_id_same_type_actors() {
        let alice = ActorId::Username(Username::from("alice").into());
        let bob = ActorId::Username(Username::from("bob").into());

        let id = ConversationId::from_pair(&alice, &bob);
        assert_eq!(id.as_str(), "u-alice+u-bob");
    }

    #[test]
    fn conversation_id_display() {
        let alice = ActorId::Username(Username::from("alice").into());
        let bob = ActorId::Username(Username::from("bob").into());

        let id = ConversationId::from_pair(&alice, &bob);
        assert_eq!(format!("{id}"), "u-alice+u-bob");
    }

    #[test]
    fn message_domain_roundtrip() {
        let msg = Message::new(
            "a-i-abcd+u-alice".to_string(),
            ActorId::Username(Username::from("alice").into()),
            "hello".to_string(),
        );

        let json = serde_json::to_string(&msg).expect("serialize");
        let decoded: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, msg);
        assert!(!decoded.deleted);
    }

    #[test]
    fn message_api_domain_roundtrip() {
        let api_msg = api::messages::Message::new(
            "a-i-abcd+u-alice".to_string(),
            ActorId::Username(Username::from("alice").into()),
            "hello".to_string(),
        );

        let domain_msg: Message = api_msg.clone().into();
        let back: api::messages::Message = domain_msg.into();

        assert_eq!(back, api_msg);
    }
}
