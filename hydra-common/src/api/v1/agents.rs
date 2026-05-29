use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

/// Validation failure for [`AgentName::try_new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentNameError {
    Empty,
    ContainsWhitespace,
    ContainsSlash,
}

impl fmt::Display for AgentNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentNameError::Empty => f.write_str("agent name must not be empty"),
            AgentNameError::ContainsWhitespace => {
                f.write_str("agent name must not contain whitespace")
            }
            AgentNameError::ContainsSlash => f.write_str("agent name must not contain '/'"),
        }
    }
}

impl std::error::Error for AgentNameError {}

/// A validated agent name (e.g. `pm`, `swe`, `reviewer`).
///
/// Introduced in the actor-system overhaul
/// (`/designs/actor-system-overhaul.md`, §3.1) as the typed counterpart
/// to free-string agent names. `AgentName` is the carrier for
/// [`crate::actor_ref::ActorId::Agent`] and
/// [`crate::principal::Principal::Agent`]. In Phase 1 it is introduced
/// as a separate newtype; Phase 2 retypes the agent-name field on
/// `Session` from `Option<String>` to `Option<AgentName>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
#[serde(transparent)]
#[non_exhaustive]
pub struct AgentName(String);

// Hand-rolled `Deserialize` so the validation in `try_new` (no
// whitespace / slash / empty) runs on the read path too. Phase 2 of
// `/designs/actor-system-overhaul.md` (§3.4) makes this the moment
// where historical malformed `Session.agent_name` values surface
// as loud deserialization errors instead of silently passing through.
impl<'de> Deserialize<'de> for AgentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        AgentName::try_new(s).map_err(serde::de::Error::custom)
    }
}

impl AgentName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validating constructor: rejects empty strings, whitespace, and
    /// `/`. New code should prefer this over any string-form construction.
    pub fn try_new(value: impl Into<String>) -> Result<Self, AgentNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(AgentNameError::Empty);
        }
        if value.chars().any(char::is_whitespace) {
            return Err(AgentNameError::ContainsWhitespace);
        }
        if value.contains('/') {
            return Err(AgentNameError::ContainsSlash);
        }
        Ok(Self(value))
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for AgentName {
    type Err = AgentNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_new(s)
    }
}

impl AsRef<str> for AgentName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

fn default_max_tries() -> i32 {
    3
}

fn default_max_simultaneous() -> i32 {
    i32::MAX
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
    #[serde(default)]
    pub mcp_config_path: Option<String>,
    #[serde(default)]
    pub mcp_config: Option<String>,
    #[serde(default = "default_max_tries")]
    pub max_tries: i32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: i32,
    #[serde(default)]
    pub is_assignment_agent: bool,
    #[serde(default)]
    pub is_default_conversation_agent: bool,
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl AgentRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        prompt: impl Into<String>,
        prompt_path: impl Into<String>,
        mcp_config_path: Option<String>,
        mcp_config: Option<String>,
        max_tries: i32,
        max_simultaneous: i32,
        is_assignment_agent: bool,
        is_default_conversation_agent: bool,
        secrets: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
            prompt_path: prompt_path.into(),
            mcp_config_path,
            mcp_config,
            max_tries,
            max_simultaneous,
            is_assignment_agent,
            is_default_conversation_agent,
            secrets,
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
    #[serde(default)]
    pub prompt_path: String,
    #[serde(default)]
    pub mcp_config_path: Option<String>,
    #[serde(default)]
    pub mcp_config: Option<String>,
    #[serde(default = "default_max_tries")]
    pub max_tries: i32,
    #[serde(default = "default_max_simultaneous")]
    pub max_simultaneous: i32,
    #[serde(default)]
    pub is_assignment_agent: bool,
    #[serde(default)]
    pub is_default_conversation_agent: bool,
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl UpsertAgentRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        prompt: impl Into<String>,
        max_tries: i32,
        max_simultaneous: i32,
        mcp_config_path: Option<String>,
        mcp_config: Option<String>,
        is_assignment_agent: bool,
        is_default_conversation_agent: bool,
        secrets: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
            prompt_path: String::new(),
            mcp_config_path,
            mcp_config,
            max_tries,
            max_simultaneous,
            is_assignment_agent,
            is_default_conversation_agent,
            secrets,
        }
    }
}

impl From<UpsertAgentRequest> for AgentRecord {
    fn from(request: UpsertAgentRequest) -> Self {
        Self {
            name: request.name,
            prompt: request.prompt,
            prompt_path: request.prompt_path,
            mcp_config_path: request.mcp_config_path,
            mcp_config: request.mcp_config,
            max_tries: request.max_tries,
            max_simultaneous: request.max_simultaneous,
            is_assignment_agent: request.is_assignment_agent,
            is_default_conversation_agent: request.is_default_conversation_agent,
            secrets: request.secrets,
        }
    }
}

impl From<AgentRecord> for UpsertAgentRequest {
    fn from(record: AgentRecord) -> Self {
        Self {
            name: record.name,
            prompt: record.prompt,
            prompt_path: record.prompt_path,
            mcp_config_path: record.mcp_config_path,
            mcp_config: record.mcp_config,
            max_tries: record.max_tries,
            max_simultaneous: record.max_simultaneous,
            is_assignment_agent: record.is_assignment_agent,
            is_default_conversation_agent: record.is_default_conversation_agent,
            secrets: record.secrets,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_record_round_trip_with_default_conversation_flag() {
        let record = AgentRecord::new(
            "swe",
            "do work",
            "/agents/swe/prompt.md",
            None,
            None,
            3,
            5,
            false,
            true,
            vec!["OPENAI_API_KEY".to_string()],
        );
        let json = serde_json::to_string(&record).unwrap();
        let parsed: AgentRecord = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_default_conversation_agent);
        assert_eq!(parsed, record);
    }

    #[test]
    fn agent_record_deserializes_without_default_conversation_flag() {
        // Older JSON omits the new field; it must still deserialize.
        let json = r#"{
            "name": "swe",
            "prompt": "",
            "prompt_path": "/agents/swe/prompt.md",
            "mcp_config_path": null,
            "mcp_config": null,
            "max_tries": 3,
            "max_simultaneous": 5,
            "is_assignment_agent": false,
            "secrets": []
        }"#;
        let parsed: AgentRecord = serde_json::from_str(json).unwrap();
        assert!(!parsed.is_default_conversation_agent);
    }

    #[test]
    fn agent_name_try_new_accepts_well_formed() {
        let n = AgentName::try_new("swe").unwrap();
        assert_eq!(n.as_str(), "swe");
        assert_eq!(n.to_string(), "swe");
    }

    #[test]
    fn agent_name_try_new_rejects_empty() {
        assert_eq!(AgentName::try_new(""), Err(AgentNameError::Empty));
    }

    #[test]
    fn agent_name_try_new_rejects_whitespace() {
        assert_eq!(
            AgentName::try_new("sw e"),
            Err(AgentNameError::ContainsWhitespace)
        );
        assert_eq!(
            AgentName::try_new("\nswe"),
            Err(AgentNameError::ContainsWhitespace)
        );
    }

    #[test]
    fn agent_name_try_new_rejects_slash() {
        assert_eq!(
            AgentName::try_new("agents/swe"),
            Err(AgentNameError::ContainsSlash)
        );
    }

    #[test]
    fn agent_name_from_str_parses_valid_input() {
        let n: AgentName = "pm".parse().unwrap();
        assert_eq!(n.as_str(), "pm");
    }

    #[test]
    fn agent_name_serde_round_trip() {
        let n = AgentName::try_new("reviewer").unwrap();
        let json = serde_json::to_string(&n).unwrap();
        assert_eq!(json, "\"reviewer\"");
        let parsed: AgentName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, n);
    }

    #[test]
    fn upsert_agent_request_deserializes_without_default_conversation_flag() {
        let json = r#"{
            "name": "swe",
            "prompt": "draft",
            "max_tries": 3,
            "max_simultaneous": 5,
            "is_assignment_agent": false
        }"#;
        let parsed: UpsertAgentRequest = serde_json::from_str(json).unwrap();
        assert!(!parsed.is_default_conversation_agent);
    }
}
