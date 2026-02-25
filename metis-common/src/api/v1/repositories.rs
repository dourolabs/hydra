use crate::RepoName;
use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !b
}

/// Configuration for a single review request entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReviewRequestConfig {
    pub assignee: String,
}

/// Configuration for the merge request issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct MergeRequestConfig {
    pub assignee: Option<String>,
}

/// Per-repo workflow configuration for patch review and merge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(default)]
pub struct RepoWorkflowConfig {
    pub review_requests: Vec<ReviewRequestConfig>,
    pub merge_request: Option<MergeRequestConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Repository {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_workflow: Option<RepoWorkflowConfig>,
}

impl Repository {
    pub fn new(
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
        patch_workflow: Option<RepoWorkflowConfig>,
    ) -> Self {
        Self {
            remote_url,
            default_branch,
            default_image,
            deleted: false,
            patch_workflow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct RepositoryRecord {
    pub name: RepoName,
    pub repository: Repository,
}

impl RepositoryRecord {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

impl From<(RepoName, Repository)> for RepositoryRecord {
    fn from((name, repository): (RepoName, Repository)) -> Self {
        Self::new(name, repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateRepositoryRequest {
    pub name: RepoName,
    #[serde(flatten)]
    pub repository: Repository,
}

impl CreateRepositoryRequest {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpdateRepositoryRequest {
    #[serde(flatten)]
    pub repository: Repository,
}

impl UpdateRepositoryRequest {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl UpsertRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchRepositoriesQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

impl SearchRepositoriesQuery {
    pub fn new(include_deleted: Option<bool>) -> Self {
        Self { include_deleted }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<RepositoryRecord>,
}

impl ListRepositoriesResponse {
    pub fn new(repositories: Vec<RepositoryRecord>) -> Self {
        Self { repositories }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DeleteRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl DeleteRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repository_without_patch_workflow_deserializes_to_none() {
        let json = json!({
            "remote_url": "https://example.com/repo.git",
            "default_branch": "main",
            "default_image": null
        });
        let repo: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(repo.patch_workflow, None);
    }

    #[test]
    fn repository_with_null_patch_workflow_deserializes_to_none() {
        let json = json!({
            "remote_url": "https://example.com/repo.git",
            "patch_workflow": null
        });
        let repo: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(repo.patch_workflow, None);
    }

    #[test]
    fn patch_workflow_round_trips_through_serde_json() {
        let config = RepoWorkflowConfig {
            review_requests: vec![
                ReviewRequestConfig {
                    assignee: "alice".to_string(),
                },
                ReviewRequestConfig {
                    assignee: "$patch_creator".to_string(),
                },
            ],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("bob".to_string()),
            }),
        };
        let repo = Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            None,
            Some(config.clone()),
        );

        let serialized = serde_json::to_value(&repo).unwrap();
        let deserialized: Repository = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized.patch_workflow, Some(config));
    }

    #[test]
    fn patch_workflow_none_is_omitted_from_serialized_json() {
        let repo = Repository::new("https://example.com/repo.git".to_string(), None, None, None);
        let serialized = serde_json::to_value(&repo).unwrap();
        assert!(
            !serialized
                .as_object()
                .unwrap()
                .contains_key("patch_workflow"),
            "patch_workflow should be omitted when None"
        );
    }

    #[test]
    fn repo_workflow_config_defaults_to_empty() {
        let config: RepoWorkflowConfig = serde_json::from_value(json!({})).unwrap();
        assert!(config.review_requests.is_empty());
        assert_eq!(config.merge_request, None);
    }

    #[test]
    fn new_with_patch_workflow_sets_field() {
        let config = RepoWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "reviewer".to_string(),
            }],
            merge_request: None,
        };
        let repo = Repository::new(
            "https://example.com/repo.git".to_string(),
            None,
            None,
            Some(config.clone()),
        );
        assert_eq!(repo.patch_workflow, Some(config));
    }
}
