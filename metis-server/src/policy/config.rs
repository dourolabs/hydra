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

/// Top-level policy configuration, deserializable from the `[policies]`
/// section of the server TOML config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    #[serde(flatten)]
    pub global: PolicyList,
    /// Per-repo policy overrides. Keys are repo names (e.g. "owner/repo").
    /// When a repo has an entry here, its policy list is used instead of
    /// the global defaults for that repo.
    pub repos: HashMap<String, PolicyList>,
}

/// Configuration for a review request issue created when a patch is opened.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewRequestConfig {
    /// Assignee for the review request issue. Supports variable substitution:
    /// `$patch_creator` resolves to the username of the patch creator at
    /// automation runtime.
    pub assignee: String,
}

/// Configuration for the merge request issue created when a patch is opened.
#[derive(Debug, Clone, Deserialize)]
pub struct MergeRequestConfig {
    /// Assignee for the merge request issue. Supports variable substitution:
    /// `$patch_creator` resolves to the username of the patch creator at
    /// automation runtime.
    pub assignee: String,
}

/// Configurable parameters for the `patch_workflow` automation.
///
/// Controls which issues are created when a patch is opened:
/// - `review_requests`: zero or more review request issues, each with a
///   configurable assignee.
/// - `merge_request`: an optional merge request issue whose assignee is also
///   configurable. When present alongside review requests, the merge request
///   issue is blocked-on all review request issues.
///
/// Assignee strings support the `$patch_creator` variable, which is resolved
/// to the username of the patch creator at automation time.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PatchWorkflowConfig {
    /// Review request issues to create for each new patch.
    pub review_requests: Vec<ReviewRequestConfig>,
    /// Optional merge request issue to create for each new patch.
    pub merge_request: Option<MergeRequestConfig>,
}
