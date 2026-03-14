use crate::domain::users::Username;
use metis_common::api::v1 as api;
use metis_common::{IssueId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorIdentity {
    User {
        username: Username,
    },
    Session {
        session_id: SessionId,
        creator: Username,
    },
    Issue {
        issue_id: IssueId,
        creator: Username,
    },
    Service {
        service_name: String,
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
            ActorIdentity::Session {
                session_id,
                creator,
            } => api::whoami::ActorIdentity::Session {
                session_id,
                creator: creator.into(),
            },
            ActorIdentity::Issue { issue_id, creator } => api::whoami::ActorIdentity::Issue {
                issue_id,
                creator: creator.into(),
            },
            ActorIdentity::Service { service_name } => {
                api::whoami::ActorIdentity::Service { service_name }
            }
        }
    }
}

impl From<WhoAmIResponse> for api::whoami::WhoAmIResponse {
    fn from(value: WhoAmIResponse) -> Self {
        api::whoami::WhoAmIResponse::new(value.actor.into())
    }
}
