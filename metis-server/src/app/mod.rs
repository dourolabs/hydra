mod app_state;

use crate::config::{ServiceSection, non_empty};
use git2::Repository;
use metis_common::{
    PatchId, RepoName,
    jobs::{Bundle, BundleSpec},
    merge_queues::MergeQueue,
};
use std::{collections::HashMap, sync::Arc};
use tempfile::TempDir;
use thiserror::Error;
use tokio::sync::RwLock;

pub use app_state::{
    AppState, CreateJobError, SetJobStatusError, UpsertIssueError, UpsertPatchError,
};

#[derive(Debug, Clone)]
pub struct ResolvedBundle {
    pub bundle: Bundle,
    pub github_token: Option<String>,
    pub default_image: Option<String>,
}

/// Connection details for a git repository remote.
#[derive(Debug, Clone)]
pub struct GitRepository {
    pub remote_url: String,
    pub default_branch: Option<String>,
    pub github_token: Option<String>,
    pub default_image: Option<String>,
}

#[allow(dead_code)]
pub struct ConnectedRepository {
    repository: Repository,
    _workdir: TempDir,
}

impl ConnectedRepository {
    #[allow(dead_code)]
    pub fn repository(&self) -> &Repository {
        &self.repository
    }
}

/// Aggregated state for repositories the service can interact with.
#[derive(Debug, Default, Clone)]
pub struct ServiceState {
    pub repositories: HashMap<RepoName, GitRepository>,
    pub merge_queues: Arc<RwLock<HashMap<RepoName, HashMap<String, MergeQueue>>>>,
}

#[derive(Debug, Error)]
pub enum BundleResolutionError {
    #[error("unknown repository '{0}'")]
    UnknownRepository(RepoName),
}

#[derive(Debug, Error)]
pub enum MergeQueueError {
    #[error("unknown repository '{0}'")]
    UnknownRepository(RepoName),
}

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum GitRepositoryError {
    #[error("failed to allocate working directory for repository")]
    TempDir(#[source] std::io::Error),
    #[error(transparent)]
    Git(#[from] git2::Error),
}

#[allow(dead_code)]
impl GitRepository {
    /// Clone the configured remote URL into a temporary workspace and return a git2 repository handle.
    pub fn connect(&self) -> Result<ConnectedRepository, GitRepositoryError> {
        let workdir = TempDir::new().map_err(GitRepositoryError::TempDir)?;
        let repository = Repository::clone(&self.remote_url, workdir.path())?;

        Ok(ConnectedRepository {
            repository,
            _workdir: workdir,
        })
    }
}

impl ServiceState {
    pub fn from_config(config: &ServiceSection) -> Self {
        let repositories = config
            .repositories
            .iter()
            .map(|(name, repo)| {
                let github_token = repo
                    .github_token
                    .as_deref()
                    .and_then(non_empty)
                    .map(str::to_owned);
                let default_branch = repo
                    .default_branch
                    .as_deref()
                    .and_then(non_empty)
                    .map(str::to_owned);

                (
                    name.clone(),
                    GitRepository {
                        remote_url: repo.remote_url.clone(),
                        default_branch,
                        github_token,
                        default_image: repo
                            .default_image
                            .as_deref()
                            .and_then(non_empty)
                            .map(str::to_owned),
                    },
                )
            })
            .collect();

        Self::with_repositories(repositories)
    }

    pub fn with_repositories(repositories: HashMap<RepoName, GitRepository>) -> Self {
        let merge_queues = repositories
            .keys()
            .map(|name| (name.clone(), HashMap::new()))
            .collect();

        Self {
            repositories,
            merge_queues: Arc::new(RwLock::new(merge_queues)),
        }
    }

    /// Resolve a BundleSpec into a concrete Bundle using server state.
    /// Returns the instantiated bundle and an optional GitHub token to surface to the worker.
    pub fn resolve_bundle_spec(
        &self,
        spec: BundleSpec,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        match spec {
            BundleSpec::None => Ok(ResolvedBundle {
                bundle: Bundle::None,
                github_token: None,
                default_image: None,
            }),
            BundleSpec::GitRepository { url, rev } => Ok(ResolvedBundle {
                bundle: Bundle::GitRepository { url, rev },
                github_token: None,
                default_image: None,
            }),
            BundleSpec::ServiceRepository { name, rev } => {
                let repo = self
                    .repositories
                    .get(&name)
                    .ok_or_else(|| BundleResolutionError::UnknownRepository(name.clone()))?;

                let resolved_rev = rev
                    .or_else(|| repo.default_branch.clone())
                    .unwrap_or_else(|| "main".to_string());

                Ok(ResolvedBundle {
                    bundle: Bundle::GitRepository {
                        url: repo.remote_url.clone(),
                        rev: resolved_rev,
                    },
                    github_token: repo.github_token.clone(),
                    default_image: repo.default_image.clone(),
                })
            }
        }
    }

    pub async fn get_merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.ensure_repository_exists(service_repo_name)?;

        let mut merge_queues = self.merge_queues.write().await;
        let repo_queues = merge_queues.entry(service_repo_name.clone()).or_default();
        let queue = repo_queues
            .entry(branch_name.to_string())
            .or_default()
            .clone();

        Ok(queue)
    }

    pub async fn add_patch_to_merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.ensure_repository_exists(service_repo_name)?;

        let mut merge_queues = self.merge_queues.write().await;
        let repo_queues = merge_queues.entry(service_repo_name.clone()).or_default();
        let queue = repo_queues.entry(branch_name.to_string()).or_default();
        queue.patches.push(patch_id);

        Ok(queue.clone())
    }

    fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), MergeQueueError> {
        if self.repositories.contains_key(name) {
            Ok(())
        } else {
            Err(MergeQueueError::UnknownRepository(name.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GitRepository;
    use anyhow::Result;
    use git2::{Commit, Oid, Repository, Signature};
    use std::{fs, path::Path};
    use tempfile::TempDir;

    #[test]
    fn connect_returns_git2_repository_for_remote_url() -> Result<()> {
        let remote_dir = TempDir::new()?;
        let remote_repo = Repository::init(remote_dir.path())?;
        let expected_head = commit_file(&remote_repo, "README.md", "hello", "init")?;

        let repository = GitRepository {
            remote_url: remote_dir
                .path()
                .to_str()
                .expect("tempdir path is valid utf-8")
                .to_string(),
            default_branch: None,
            github_token: None,
            default_image: None,
        };

        let connected = repository.connect()?;
        let repo = connected.repository();

        assert_eq!(repo.head()?.target(), Some(expected_head));
        let origin = repo.find_remote("origin")?;
        assert_eq!(origin.url(), Some(remote_dir.path().to_str().unwrap()));

        Ok(())
    }

    fn commit_file(repo: &Repository, name: &str, contents: &str, message: &str) -> Result<Oid> {
        let signature = Signature::now("Tester", "tester@example.com")?;
        let workdir = repo
            .workdir()
            .expect("repository should be a working tree")
            .to_path_buf();
        let relative = Path::new(name);
        let full_path = workdir.join(relative);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, contents)?;

        let mut index = repo.index()?;
        index.add_path(relative)?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        let head_commit = repo.head().ok().and_then(|reference| {
            reference
                .target()
                .and_then(|target| repo.find_commit(target).ok())
        });

        let parents: Vec<&Commit> = head_commit.iter().collect();

        let commit_id = repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;

        Ok(commit_id)
    }
}
