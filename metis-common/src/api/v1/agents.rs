use serde::{Deserialize, Serialize};

fn default_max_tries() -> u32 {
    3
}

fn default_max_simultaneous() -> u32 {
    u32::MAX
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AgentRecord {
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default = "default_max_tries")]
    pub max_tries: u32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: u32,
}

impl AgentRecord {
    pub fn new(
        name: impl Into<String>,
        prompt: impl Into<String>,
        max_tries: u32,
        max_simultaneous: u32,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
            max_tries,
            max_simultaneous,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertAgentRequest {
    pub name: String,
    pub prompt: String,
    #[serde(default = "default_max_tries")]
    pub max_tries: u32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: u32,
}

impl UpsertAgentRequest {
    pub fn new(
        name: impl Into<String>,
        prompt: impl Into<String>,
        max_tries: u32,
        max_simultaneous: u32,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
            max_tries,
            max_simultaneous,
        }
    }
}

impl From<UpsertAgentRequest> for AgentRecord {
    fn from(request: UpsertAgentRequest) -> Self {
        Self {
            name: request.name,
            prompt: request.prompt,
            max_tries: request.max_tries,
            max_simultaneous: request.max_simultaneous,
        }
    }
}

impl From<AgentRecord> for UpsertAgentRequest {
    fn from(record: AgentRecord) -> Self {
        Self {
            name: record.name,
            prompt: record.prompt,
            max_tries: record.max_tries,
            max_simultaneous: record.max_simultaneous,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AgentResponse {
    pub agent: AgentRecord,
}

impl AgentResponse {
    pub fn new(agent: AgentRecord) -> Self {
        Self { agent }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DeleteAgentResponse {
    pub agent: AgentRecord,
}

impl DeleteAgentResponse {
    pub fn new(agent: AgentRecord) -> Self {
        Self { agent }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListAgentsResponse {
    pub agents: Vec<AgentRecord>,
}

impl ListAgentsResponse {
    pub fn new(agents: Vec<AgentRecord>) -> Self {
        Self { agents }
    }
}
