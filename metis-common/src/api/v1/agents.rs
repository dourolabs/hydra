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
    #[serde(default)]
    pub prompt_path: String,
    #[serde(default = "default_max_tries")]
    pub max_tries: u32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: u32,
    #[serde(default)]
    pub is_assignment_agent: bool,
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
            prompt_path: String::new(),
            max_tries,
            max_simultaneous,
            is_assignment_agent: false,
        }
    }

    pub fn with_prompt_path(mut self, prompt_path: impl Into<String>) -> Self {
        self.prompt_path = prompt_path.into();
        self
    }

    pub fn with_is_assignment_agent(mut self, is_assignment_agent: bool) -> Self {
        self.is_assignment_agent = is_assignment_agent;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertAgentRequest {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub prompt_path: String,
    #[serde(default = "default_max_tries")]
    pub max_tries: u32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: u32,
    #[serde(default)]
    pub is_assignment_agent: bool,
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
            prompt_path: String::new(),
            max_tries,
            max_simultaneous,
            is_assignment_agent: false,
        }
    }
}

impl From<UpsertAgentRequest> for AgentRecord {
    fn from(request: UpsertAgentRequest) -> Self {
        Self {
            name: request.name,
            prompt: request.prompt,
            prompt_path: request.prompt_path,
            max_tries: request.max_tries,
            max_simultaneous: request.max_simultaneous,
            is_assignment_agent: request.is_assignment_agent,
        }
    }
}

impl From<AgentRecord> for UpsertAgentRequest {
    fn from(record: AgentRecord) -> Self {
        Self {
            name: record.name,
            prompt: record.prompt,
            prompt_path: record.prompt_path,
            max_tries: record.max_tries,
            max_simultaneous: record.max_simultaneous,
            is_assignment_agent: record.is_assignment_agent,
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
