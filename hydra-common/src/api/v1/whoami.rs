use crate::api::v1::agents::AgentName;
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
    /// Phase 2 of the actor-system overhaul
    /// (`/designs/actor-system-overhaul.md` §3.4): once
    /// `create_actor_for_job` routes through `actor_id_of(session)`,
    /// agent-spawned sessions surface here as `Agent { name, creator }`
    /// instead of the legacy `Session` / `Issue` variants. `creator`
    /// is the human on whose behalf the agent ran (the session's
    /// creator), so CLI clients running inside agent jobs can keep
    /// resolving "who am I acting as" without a separate lookup.
    Agent {
        name: AgentName,
        creator: Username,
    },
    /// Ad-hoc sessions (created outside the agent system) — design
    /// §3.4. The post-Phase-2 replacement for `ActorId::Session` on
    /// the session-actor path.
    Adhoc {
        session_id: SessionId,
        creator: Username,
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
