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

    /// Parse a GitHub remote URL to extract owner and repo name.
    ///
    /// Supports HTTPS (`https://github.com/owner/repo[.git]`) and
    /// SSH (`git@github.com:owner/repo[.git]`) formats.
    /// Returns `None` for non-GitHub URLs.
    ///
    /// All code that needs to determine whether a repository is hosted on GitHub
    /// (or extract owner/repo info) should use this method or [`is_github`](Self::is_github)
    /// rather than ad-hoc string matching on the remote URL.
    pub fn github_owner_repo(&self) -> Option<(String, String)> {
        let remote_url = &self.remote_url;

        // HTTPS: https://github.com/owner/repo.git or https://github.com/owner/repo
        if let Some(path) = remote_url
            .strip_prefix("https://github.com/")
            .or_else(|| remote_url.strip_prefix("http://github.com/"))
        {
            let path = path.trim_end_matches('/').trim_end_matches(".git");
            let (owner, repo) = path.split_once('/')?;
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some((owner.to_string(), repo.to_string()));
        }

        // SSH: git@github.com:owner/repo.git
        if let Some(path) = remote_url.strip_prefix("git@github.com:") {
            let path = path.trim_end_matches('/').trim_end_matches(".git");
            let (owner, repo) = path.split_once('/')?;
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some((owner.to_string(), repo.to_string()));
        }

        None
    }

    /// Returns `true` if this repository is hosted on GitHub.
    pub fn is_github(&self) -> bool {
        self.github_owner_repo().is_some()
    }

    /// Returns `true` if this repository is a local filesystem path.
    ///
    /// Detects `file://` URLs and absolute paths starting with `/`.
    pub fn is_local(&self) -> bool {
        self.remote_url.starts_with("file://") || self.remote_url.starts_with('/')
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
    fn github_owner_repo_https_with_git_suffix() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_https_without_git_suffix() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh() {
        let repo = Repository::new(
            "git@github.com:dourolabs/hydra.git".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh_without_git_suffix() {
        let repo = Repository::new(
            "git@github.com:dourolabs/hydra".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(
            repo.github_owner_repo(),
            Some(("dourolabs".to_string(), "hydra".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_non_github() {
        let repo = Repository::new(
            "https://gitlab.com/org/repo.git".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(repo.github_owner_repo(), None);
    }

    #[test]
    fn github_owner_repo_file_url() {
        let repo = Repository::new("file:///home/user/repo".to_string(), None, None, None);
        assert_eq!(repo.github_owner_repo(), None);
    }

    #[test]
    fn github_owner_repo_empty_segments() {
        let repo = Repository::new("https://github.com//repo.git".to_string(), None, None, None);
        assert_eq!(repo.github_owner_repo(), None);

        let repo2 = Repository::new("https://github.com/owner/".to_string(), None, None, None);
        assert_eq!(repo2.github_owner_repo(), None);
    }

    #[test]
    fn is_github_returns_true_for_github_url() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
            None,
        );
        assert!(repo.is_github());
    }

    #[test]
    fn is_github_returns_false_for_non_github_url() {
        let repo = Repository::new(
            "https://gitlab.com/org/repo.git".to_string(),
            None,
            None,
            None,
        );
        assert!(!repo.is_github());
    }

    #[test]
    fn is_local_file_url() {
        let repo = Repository::new("file:///home/user/repo".to_string(), None, None, None);
        assert!(repo.is_local());
    }

    #[test]
    fn is_local_absolute_path() {
        let repo = Repository::new("/home/user/repo".to_string(), None, None, None);
        assert!(repo.is_local());
    }

    #[test]
    fn is_local_returns_false_for_github() {
        let repo = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            None,
            None,
            None,
        );
        assert!(!repo.is_local());
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
