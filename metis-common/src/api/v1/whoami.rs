use crate::api::v1::users::Username;
use crate::{IssueId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActorIdentity {
    User {
        username: Username,
    },
    #[serde(alias = "task")]
    Session {
        #[serde(alias = "task_id")]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WhoAmIResponse {
    pub actor: ActorIdentity,
}

impl WhoAmIResponse {
    pub fn new(actor: ActorIdentity) -> Self {
        Self { actor }
    }
}
