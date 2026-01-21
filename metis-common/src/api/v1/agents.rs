use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentRecord {
    pub name: String,
}

impl AgentRecord {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListAgentsResponse {
    pub agents: Vec<AgentRecord>,
}

impl ListAgentsResponse {
    pub fn new(agents: Vec<AgentRecord>) -> Self {
        Self { agents }
    }
}
