mod app_state;

use crate::{
    config::{ServiceSection, non_empty},
    merge_queue::MergeQueueImpl,
    store::StoreError,
};
use git2::Repository;
use metis_common::{
    PatchId, RepoName,
    jobs::{Bundle, BundleSpec},
    merge_queues::MergeQueue,
    patches::Patch,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tempfile::TempDir;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

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
    pub merge_queues: Arc<RwLock<HashMap<RepoName, HashMap<String, MergeQueueImpl>>>>,
    pub git_cache: Arc<RwLock<HashMap<RepoName, Arc<Mutex<CachedRepository>>>>>,
}

#[derive(Debug)]
pub struct CachedRepository {
    path: PathBuf,
    _workdir: TempDir,
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
    #[error("patch '{patch_id}' not found")]
    PatchNotFound { patch_id: PatchId },
    #[error("patch '{patch_id}' targets repository '{patch_repo}' instead of '{service_repo}'")]
    PatchRepositoryMismatch {
        patch_id: PatchId,
        patch_repo: RepoName,
        service_repo: RepoName,
    },
    #[error("failed to load patch '{patch_id}'")]
    PatchLookup {
        patch_id: PatchId,
        #[source]
        source: StoreError,
    },
    #[error("failed to refresh git cache for repository '{repo_name}'")]
    Git {
        repo_name: RepoName,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to initialize merge queue for '{repo_name}' on branch '{branch_name}'")]
    QueueInitialization {
        repo_name: RepoName,
        branch_name: String,
        #[source]
        source: git2::Error,
    },
    #[error(
        "failed to append patch '{patch_id}' to merge queue for '{repo_name}' on branch '{branch_name}'"
    )]
    QueueUpdate {
        patch_id: PatchId,
        repo_name: RepoName,
        branch_name: String,
        #[source]
        source: crate::merge_queue::MergeQueueError,
    },
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

#[allow(clippy::result_large_err)]
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
            git_cache: Arc::new(RwLock::new(HashMap::new())),
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

        let merge_queues = self.merge_queues.read().await;
        let queue = merge_queues
            .get(service_repo_name)
            .and_then(|repo_queues| repo_queues.get(branch_name))
            .map(merge_queue_response)
            .unwrap_or_default();

        Ok(queue)
    }

    pub async fn add_patch_to_merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
        patch: &Patch,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.ensure_repository_exists(service_repo_name)?;

        if patch.service_repo_name != *service_repo_name {
            return Err(MergeQueueError::PatchRepositoryMismatch {
                patch_id,
                patch_repo: patch.service_repo_name.clone(),
                service_repo: service_repo_name.clone(),
            });
        }

        let repository = self.refresh_repository(service_repo_name).await?;

        let mut merge_queues = self.merge_queues.write().await;
        let repo_queues = merge_queues.entry(service_repo_name.clone()).or_default();
        let branch_name = branch_name.to_string();
        let queue_patch_id = patch_id.clone();
        let queue = match repo_queues.get_mut(&branch_name) {
            Some(queue) => queue,
            None => {
                let queue = MergeQueueImpl::new(&repository, branch_ref(&branch_name)).map_err(
                    |source| MergeQueueError::QueueInitialization {
                        repo_name: service_repo_name.clone(),
                        branch_name: branch_name.clone(),
                        source,
                    },
                )?;
                repo_queues.insert(branch_name.clone(), queue);
                repo_queues
                    .get_mut(&branch_name)
                    .expect("queue should exist after insertion")
            }
        };
        queue
            .try_squash_append_diff(
                &repository,
                queue_patch_id.clone(),
                &patch.diff,
                Some(&patch.title),
            )
            .map_err(|source| MergeQueueError::QueueUpdate {
                patch_id: queue_patch_id,
                repo_name: service_repo_name.clone(),
                branch_name: branch_name.clone(),
                source,
            })?;

        Ok(merge_queue_response(queue))
    }

    fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), MergeQueueError> {
        if self.repositories.contains_key(name) {
            Ok(())
        } else {
            Err(MergeQueueError::UnknownRepository(name.clone()))
        }
    }

    async fn refresh_repository(&self, name: &RepoName) -> Result<Repository, MergeQueueError> {
        let cache_entry = {
            let mut git_cache = self.git_cache.write().await;
            if let Some(entry) = git_cache.get(name) {
                entry.clone()
            } else {
                let repo_cfg = self
                    .repositories
                    .get(name)
                    .expect("refresh_repository called after ensure_repository_exists");
                let ConnectedRepository {
                    repository: _,
                    _workdir,
                } = repo_cfg.connect().map_err(|source| MergeQueueError::Git {
                    repo_name: name.clone(),
                    source: source.into(),
                })?;
                let cached = Arc::new(Mutex::new(CachedRepository {
                    path: _workdir.path().to_path_buf(),
                    _workdir,
                }));
                git_cache.insert(name.clone(), cached.clone());
                cached
            }
        };

        let cached = cache_entry.lock().await;
        let repository = Repository::open(&cached.path).map_err(|source| MergeQueueError::Git {
            repo_name: name.clone(),
            source: source.into(),
        })?;
        drop(cached);

        let mut remote =
            repository
                .find_remote("origin")
                .map_err(|source| MergeQueueError::Git {
                    repo_name: name.clone(),
                    source: source.into(),
                })?;
        remote
            .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
            .map_err(|source| MergeQueueError::Git {
                repo_name: name.clone(),
                source: source.into(),
            })?;

        drop(remote);
        Ok(repository)
    }
}

fn merge_queue_response(queue: &MergeQueueImpl) -> MergeQueue {
    MergeQueue {
        patches: queue
            .patches()
            .iter()
            .map(|entry| entry.patch_id.clone())
            .collect(),
    }
}

fn branch_ref(branch_name: &str) -> String {
    format!("refs/remotes/origin/{branch_name}")
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
