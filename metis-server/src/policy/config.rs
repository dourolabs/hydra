use serde::Deserialize;
use std::collections::HashMap;

/// A single policy entry in the config, consisting of a name and optional
/// parameters (as raw TOML values for the policy constructor to interpret).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PolicyEntry {
    /// Just the policy name with no params.
    Name(String),
    /// A policy with name and parameters.
    WithParams { name: String, params: toml::Value },
}

impl PolicyEntry {
    /// Returns the policy name.
    pub fn name(&self) -> &str {
        match self {
            PolicyEntry::Name(name) => name,
            PolicyEntry::WithParams { name, .. } => name,
        }
    }

    /// Returns the policy parameters, if any.
    pub fn params(&self) -> Option<&toml::Value> {
        match self {
            PolicyEntry::Name(_) => None,
            PolicyEntry::WithParams { params, .. } => Some(params),
        }
    }
}

/// Policy configuration for a single scope (global or per-repo).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PolicyList {
    pub restrictions: Vec<PolicyEntry>,
    pub automations: Vec<PolicyEntry>,
}

/// Per-repo override configuration. Only restriction overrides are supported;
/// automations always run from the global engine because the event bus does
/// not have a per-repo scope.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RepoOverride {
    pub restrictions: Vec<PolicyEntry>,
}

/// Top-level policy configuration, deserializable from the `[policies]`
/// section of the server TOML config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    #[serde(flatten)]
    pub global: PolicyList,

    /// Per-repo restriction overrides. Key is the repo name (e.g., "dourolabs/metis").
    /// Only restrictions can be overridden per-repo; automations always use the
    /// global list.
    #[serde(default)]
    pub repos: HashMap<String, RepoOverride>,
}
