mod app_state;
mod resolved_task;

use crate::{
    config::{ServiceSection, non_empty},
    domain::{
        jobs::{Bundle, BundleSpec},
        patches::Patch,
    },
    merge_queue::MergeQueueImpl,
    store::StoreError,
};
use git2::Repository;
use metis_common::{PatchId, RepoName, merge_queues::MergeQueue};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tempfile::TempDir;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

pub use app_state::{
    AppState, CreateJobError, SetJobStatusError, UpdateTodoListError, UpsertIssueError,
    UpsertPatchError,
};
pub use metis_common::repositories::{
    ServiceRepository, ServiceRepositoryConfig, ServiceRepositoryInfo,
};
pub use resolved_task::{ResolvedTask, TaskExt, TaskResolutionError};

#[derive(Debug, Clone)]
pub struct ResolvedBundle {
    pub bundle: Bundle,
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
    pub repositories: Arc<RwLock<HashMap<RepoName, ServiceRepository>>>,
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
    #[error("unsupported bundle specification")]
    UnsupportedBundleSpec,
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

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("repository '{0}' already exists")]
    AlreadyExists(RepoName),
    #[error("repository '{0}' not found")]
    NotFound(RepoName),
    #[error("failed to refresh repository '{repo_name}'")]
    Git {
        repo_name: RepoName,
        #[source]
        source: MergeQueueError,
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

fn connect_repository(repo: &ServiceRepository) -> Result<ConnectedRepository, GitRepositoryError> {
    let workdir = TempDir::new().map_err(GitRepositoryError::TempDir)?;
    let repository = Repository::clone(&repo.remote_url, workdir.path())?;

    Ok(ConnectedRepository {
        repository,
        _workdir: workdir,
    })
}

#[allow(clippy::result_large_err)]
impl ServiceState {
    pub fn from_config(config: &ServiceSection) -> Self {
        let repositories = config
            .repositories
            .iter()
            .map(|(name, repo)| {
                let default_branch = repo
                    .default_branch
                    .as_deref()
                    .and_then(non_empty)
                    .map(str::to_owned);

                (
                    name.clone(),
                    ServiceRepository::new(
                        name.clone(),
                        repo.remote_url.clone(),
                        default_branch,
                        repo.default_image
                            .as_deref()
                            .and_then(non_empty)
                            .map(str::to_owned),
                    ),
                )
            })
            .collect();

        Self::with_repositories(repositories)
    }

    pub fn with_repositories(repositories: HashMap<RepoName, ServiceRepository>) -> Self {
        let repositories: HashMap<RepoName, ServiceRepository> = repositories
            .into_iter()
            .map(|(name, mut repository)| {
                repository.name = name.clone();
                (name, repository)
            })
            .collect();
        let merge_queues = repositories
            .keys()
            .map(|name| (name.clone(), HashMap::new()))
            .collect();

        Self {
            repositories: Arc::new(RwLock::new(repositories)),
            merge_queues: Arc::new(RwLock::new(merge_queues)),
            git_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn list_repository_info(&self) -> Vec<ServiceRepositoryInfo> {
        let repositories = self.repositories.read().await;

        repositories
            .values()
            .map(ServiceRepository::without_secret)
            .collect()
    }

    pub async fn repository(&self, name: &RepoName) -> Option<ServiceRepository> {
        let repositories = self.repositories.read().await;

        repositories.get(name).cloned()
    }

    pub async fn has_repository(&self, name: &RepoName) -> bool {
        let repositories = self.repositories.read().await;

        repositories.contains_key(name)
    }

    pub async fn create_repository(
        &self,
        repository: ServiceRepository,
    ) -> Result<ServiceRepository, RepositoryError> {
        let name = repository.name.clone();

        {
            let mut repositories = self.repositories.write().await;

            if repositories.contains_key(&name) {
                return Err(RepositoryError::AlreadyExists(name));
            }

            repositories.insert(name.clone(), repository);
        }

        if let Err(err) = self.refresh_repository(&name).await {
            {
                let mut repositories = self.repositories.write().await;
                repositories.remove(&name);
            }
            let mut git_cache = self.git_cache.write().await;
            git_cache.remove(&name);

            return Err(RepositoryError::Git {
                repo_name: name,
                source: err,
            });
        }

        self.initialize_merge_queue(&name).await;

        let repositories = self.repositories.read().await;
        Ok(repositories
            .get(&name)
            .cloned()
            .expect("repository should exist after creation"))
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<ServiceRepository, RepositoryError> {
        let previous = {
            let mut repositories = self.repositories.write().await;

            let Some(repository) = repositories.get_mut(&name) else {
                return Err(RepositoryError::NotFound(name));
            };

            let previous = repository.clone();
            repository.remote_url = config.remote_url;
            repository.default_branch = config.default_branch;
            repository.default_image = config.default_image;
            previous
        };

        let previous_merge_queues = {
            let mut merge_queues = self.merge_queues.write().await;
            merge_queues.insert(name.clone(), HashMap::new())
        };
        let previous_cache = {
            let mut git_cache = self.git_cache.write().await;
            git_cache.remove(&name)
        };

        if let Err(err) = self.refresh_repository(&name).await {
            {
                let mut repositories = self.repositories.write().await;
                repositories.insert(name.clone(), previous);
            }

            {
                let mut merge_queues = self.merge_queues.write().await;
                if let Some(existing) = previous_merge_queues {
                    merge_queues.insert(name.clone(), existing);
                } else {
                    merge_queues.remove(&name);
                }
            }

            if let Some(cache) = previous_cache {
                let mut git_cache = self.git_cache.write().await;
                git_cache.insert(name.clone(), cache);
            }

            return Err(RepositoryError::Git {
                repo_name: name,
                source: err,
            });
        }

        let repositories = self.repositories.read().await;
        Ok(repositories
            .get(&name)
            .cloned()
            .expect("repository should exist after update"))
    }

    async fn initialize_merge_queue(&self, name: &RepoName) {
        let mut merge_queues = self.merge_queues.write().await;
        merge_queues.insert(name.clone(), HashMap::new());
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn cached_repository_paths(&self) -> HashMap<RepoName, PathBuf> {
        let git_cache = self.git_cache.read().await;
        let mut repositories = HashMap::new();

        for (name, repo) in git_cache.iter() {
            let path = repo.lock().await.path.clone();
            repositories.insert(name.clone(), path);
        }

        repositories
    }

    /// Resolve a BundleSpec into a concrete Bundle using server state.
    /// Returns the instantiated bundle and an optional GitHub token to surface to the worker.
    pub async fn resolve_bundle_spec(
        &self,
        spec: BundleSpec,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        match spec {
            BundleSpec::None => Ok(ResolvedBundle {
                bundle: Bundle::None,
                default_image: None,
            }),
            BundleSpec::GitRepository { url, rev } => Ok(ResolvedBundle {
                bundle: Bundle::GitRepository { url, rev },
                default_image: None,
            }),
            BundleSpec::ServiceRepository { name, rev } => {
                let repositories = self.repositories.read().await;
                let repo = repositories
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
        self.ensure_repository_exists(service_repo_name).await?;

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
        self.ensure_repository_exists(service_repo_name).await?;

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

    async fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), MergeQueueError> {
        let repositories = self.repositories.read().await;

        if repositories.contains_key(name) {
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
                let repo_cfg = {
                    let repositories = self.repositories.read().await;
                    repositories
                        .get(name)
                        .expect("refresh_repository called after ensure_repository_exists")
                        .clone()
                };
                let ConnectedRepository {
                    repository: _,
                    _workdir,
                } = connect_repository(&repo_cfg).map_err(|source| MergeQueueError::Git {
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
    MergeQueue::new(
        queue
            .patches()
            .iter()
            .map(|entry| entry.patch_id.clone())
            .collect(),
    )
}

fn branch_ref(branch_name: &str) -> String {
    format!("refs/remotes/origin/{branch_name}")
}

#[cfg(test)]
mod tests {
    use super::{ServiceRepository, connect_repository};
    use anyhow::Result;
    use git2::{Commit, Oid, Repository, Signature};
    use metis_common::RepoName;
    use std::{fs, path::Path, str::FromStr};
    use tempfile::TempDir;

    #[test]
    fn connect_returns_git2_repository_for_remote_url() -> Result<()> {
        let remote_dir = TempDir::new()?;
        let remote_repo = Repository::init(remote_dir.path())?;
        let expected_head = commit_file(&remote_repo, "README.md", "hello", "init")?;

        let repository = ServiceRepository::new(
            RepoName::from_str("dourolabs/metis")?,
            remote_dir
                .path()
                .to_str()
                .expect("tempdir path is valid utf-8")
                .to_string(),
            None,
            None,
        );

        let connected = connect_repository(&repository)?;
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
