use crate::domain::users::Username;
use metis_common::TaskId;
use metis_common::api::v1 as api;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorIdentity {
    User { username: Username },
    Task { task_id: TaskId },
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
            ActorIdentity::Task { task_id } => api::whoami::ActorIdentity::Task { task_id },
        }
    }
}

impl From<WhoAmIResponse> for api::whoami::WhoAmIResponse {
    fn from(value: WhoAmIResponse) -> Self {
        api::whoami::WhoAmIResponse::new(value.actor.into())
    }
}
