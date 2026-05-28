use crate::domain::users::Username;
use hydra_common::SessionId;
use hydra_common::api::v1 as api;
use hydra_common::api::v1::agents::AgentName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorIdentity {
    User {
        username: Username,
    },
    Agent {
        name: AgentName,
        creator: Username,
    },
    Adhoc {
        session_id: SessionId,
        creator: Username,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhoAmIResponse {
    pub actor: ActorIdentity,
}

impl WhoAmIResponse {
    pub fn new(actor: ActorIdentity) -> Self {
        Self { actor }
    }
}

impl From<ActorIdentity> for api::whoami::ActorIdentity {
    fn from(value: ActorIdentity) -> Self {
        match value {
            ActorIdentity::User { username } => api::whoami::ActorIdentity::User {
                username: username.into(),
            },
            ActorIdentity::Agent { name, creator } => api::whoami::ActorIdentity::Agent {
                name,
                creator: creator.into(),
            },
            ActorIdentity::Adhoc {
                session_id,
                creator,
            } => api::whoami::ActorIdentity::Adhoc {
                session_id,
                creator: creator.into(),
            },
        }
    }
}

impl From<WhoAmIResponse> for api::whoami::WhoAmIResponse {
    fn from(value: WhoAmIResponse) -> Self {
        api::whoami::WhoAmIResponse::new(value.actor.into())
    }
}
