mod app_state;
mod resolved_task;

use crate::{
    config::{ServiceSection, non_empty},
    domain::{
        jobs::{Bundle, BundleSpec},
        patches::Patch,
    },
    merge_queue::MergeQueueImpl,
    store::{Store, StoreError},
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
    #[error("failed to load repository '{repo_name}'")]
    RepositoryLookup {
        repo_name: RepoName,
        #[source]
        source: StoreError,
    },
    #[error("unsupported bundle specification")]
    UnsupportedBundleSpec,
}

#[derive(Debug, Error)]
pub enum MergeQueueError {
    #[error("unknown repository '{0}'")]
    UnknownRepository(RepoName),
    #[error("failed to load repository '{repo_name}'")]
    RepositoryLookup {
        repo_name: RepoName,
        #[source]
        source: StoreError,
    },
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
    #[error("failed to persist repository '{repo_name}'")]
    Store {
        repo_name: RepoName,
        #[source]
        source: StoreError,
    },
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
    pub fn with_repository_names<I>(names: I) -> Self
    where
        I: IntoIterator<Item = RepoName>,
    {
        let merge_queues = names
            .into_iter()
            .map(|name| (name, HashMap::new()))
            .collect();

        Self {
            merge_queues: Arc::new(RwLock::new(merge_queues)),
            git_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn for_store(store: &dyn Store) -> Result<Self, StoreError> {
        let repositories = store.list_repositories().await?;
        Ok(Self::with_repository_names(
            repositories.into_iter().map(|(name, _)| name),
        ))
    }

    pub async fn list_repository_info(
        &self,
        store: &dyn Store,
    ) -> Result<Vec<ServiceRepositoryInfo>, StoreError> {
        let repositories = store.list_repositories().await?;

        Ok(repositories
            .into_iter()
            .map(|(name, config)| ServiceRepository::from((name, config)).without_secret())
            .collect())
    }

    pub async fn repository(
        &self,
        store: &dyn Store,
        name: &RepoName,
    ) -> Result<ServiceRepository, StoreError> {
        repository_from_store(store, name).await
    }

    pub async fn has_repository(
        &self,
        store: &dyn Store,
        name: &RepoName,
    ) -> Result<bool, StoreError> {
        match store.get_repository(name).await {
            Ok(_) => Ok(true),
            Err(StoreError::RepositoryNotFound(_)) => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub async fn create_repository(
        &self,
        store: &mut dyn Store,
        repository: ServiceRepository,
    ) -> Result<ServiceRepository, RepositoryError> {
        let name = repository.name.clone();

        match store.get_repository(&name).await {
            Ok(_) => return Err(RepositoryError::AlreadyExists(name)),
            Err(StoreError::RepositoryNotFound(_)) => {}
            Err(source) => {
                return Err(RepositoryError::Store {
                    repo_name: name,
                    source,
                });
            }
        }

        if let Err(err) = self.refresh_repository(&repository).await {
            self.cleanup_repository_state(&name).await;
            return Err(RepositoryError::Git {
                repo_name: name,
                source: err,
            });
        }

        if let Err(source) = store
            .add_repository(
                name.clone(),
                ServiceRepositoryConfig::new(
                    repository.remote_url.clone(),
                    repository.default_branch.clone(),
                    repository.default_image.clone(),
                ),
            )
            .await
        {
            self.cleanup_repository_state(&name).await;
            return match source {
                StoreError::RepositoryAlreadyExists(_) => Err(RepositoryError::AlreadyExists(name)),
                other => Err(RepositoryError::Store {
                    repo_name: name,
                    source: other,
                }),
            };
        }

        self.initialize_merge_queue(&name).await;

        Ok(repository)
    }

    pub async fn update_repository(
        &self,
        store: &mut dyn Store,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<ServiceRepository, RepositoryError> {
        let previous_config = match store.get_repository(&name).await {
            Ok(config) => config,
            Err(StoreError::RepositoryNotFound(_)) => return Err(RepositoryError::NotFound(name)),
            Err(source) => {
                return Err(RepositoryError::Store {
                    repo_name: name,
                    source,
                });
            }
        };

        let repository = ServiceRepository::from((name.clone(), config.clone()));

        let previous_merge_queues = {
            let mut merge_queues = self.merge_queues.write().await;
            merge_queues.insert(name.clone(), HashMap::new())
        };
        let previous_cache = {
            let mut git_cache = self.git_cache.write().await;
            git_cache.remove(&name)
        };

        if let Err(err) = self.refresh_repository(&repository).await {
            self.restore_repository_state(&name, previous_merge_queues, previous_cache)
                .await;
            return Err(RepositoryError::Git {
                repo_name: name,
                source: err,
            });
        }

        if let Err(source) = store.update_repository(name.clone(), config).await {
            self.restore_repository_state(&name, previous_merge_queues, previous_cache)
                .await;
            if let Err(restore_err) = store
                .update_repository(name.clone(), previous_config.clone())
                .await
            {
                tracing::warn!(
                    repository = %name,
                    error = %restore_err,
                    "failed to restore previous repository config after update failure"
                );
            }
            return Err(match source {
                StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name),
                other => RepositoryError::Store {
                    repo_name: name,
                    source: other,
                },
            });
        }

        Ok(repository)
    }

    async fn restore_repository_state(
        &self,
        name: &RepoName,
        previous_merge_queues: Option<HashMap<String, MergeQueueImpl>>,
        previous_cache: Option<Arc<Mutex<CachedRepository>>>,
    ) {
        let mut merge_queues = self.merge_queues.write().await;
        if let Some(queues) = previous_merge_queues {
            merge_queues.insert(name.clone(), queues);
        } else {
            merge_queues.remove(name);
        }

        let mut git_cache = self.git_cache.write().await;
        if let Some(cache) = previous_cache {
            git_cache.insert(name.clone(), cache);
        }
    }

    async fn cleanup_repository_state(&self, name: &RepoName) {
        let mut merge_queues = self.merge_queues.write().await;
        merge_queues.remove(name);

        let mut git_cache = self.git_cache.write().await;
        git_cache.remove(name);
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
        store: &dyn Store,
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
                let repo = repository_from_store(store, &name)
                    .await
                    .map_err(|err| match err {
                        StoreError::RepositoryNotFound(_) => {
                            BundleResolutionError::UnknownRepository(name.clone())
                        }
                        other => BundleResolutionError::RepositoryLookup {
                            repo_name: name.clone(),
                            source: other,
                        },
                    })?;

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
        store: &dyn Store,
        service_repo_name: &RepoName,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.repository_for_merge_queue(store, service_repo_name)
            .await?;

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
        store: &dyn Store,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
        patch: &Patch,
    ) -> Result<MergeQueue, MergeQueueError> {
        let repository = self
            .repository_for_merge_queue(store, service_repo_name)
            .await?;

        if patch.service_repo_name != *service_repo_name {
            return Err(MergeQueueError::PatchRepositoryMismatch {
                patch_id,
                patch_repo: patch.service_repo_name.clone(),
                service_repo: service_repo_name.clone(),
            });
        }

        let repository = self.refresh_repository(&repository).await?;

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

    async fn repository_for_merge_queue(
        &self,
        store: &dyn Store,
        name: &RepoName,
    ) -> Result<ServiceRepository, MergeQueueError> {
        repository_from_store(store, name)
            .await
            .map_err(|err| match err {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: name.clone(),
                    source: other,
                },
            })
    }

    async fn refresh_repository(
        &self,
        service_repo: &ServiceRepository,
    ) -> Result<Repository, MergeQueueError> {
        let cache_entry = {
            let mut git_cache = self.git_cache.write().await;
            if let Some(entry) = git_cache.get(&service_repo.name) {
                entry.clone()
            } else {
                let ConnectedRepository {
                    repository: _,
                    _workdir,
                } = connect_repository(service_repo).map_err(|source| MergeQueueError::Git {
                    repo_name: service_repo.name.clone(),
                    source: source.into(),
                })?;
                let cached = Arc::new(Mutex::new(CachedRepository {
                    path: _workdir.path().to_path_buf(),
                    _workdir,
                }));
                git_cache.insert(service_repo.name.clone(), cached.clone());
                cached
            }
        };

        let cached = cache_entry.lock().await;
        let repository = Repository::open(&cached.path).map_err(|source| MergeQueueError::Git {
            repo_name: service_repo.name.clone(),
            source: source.into(),
        })?;
        drop(cached);

        let mut remote =
            repository
                .find_remote("origin")
                .map_err(|source| MergeQueueError::Git {
                    repo_name: service_repo.name.clone(),
                    source: source.into(),
                })?;
        remote
            .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
            .map_err(|source| MergeQueueError::Git {
                repo_name: service_repo.name.clone(),
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

pub async fn sync_repositories_from_config(
    store: &mut dyn Store,
    config: &ServiceSection,
) -> Result<(), StoreError> {
    for (name, repository) in &config.repositories {
        let normalized = ServiceRepositoryConfig::new(
            repository.remote_url.clone(),
            repository
                .default_branch
                .as_deref()
                .and_then(non_empty)
                .map(str::to_owned),
            repository
                .default_image
                .as_deref()
                .and_then(non_empty)
                .map(str::to_owned),
        );

        match store.add_repository(name.clone(), normalized.clone()).await {
            Ok(()) => {}
            Err(StoreError::RepositoryAlreadyExists(_)) => {
                store.update_repository(name.clone(), normalized).await?;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

async fn repository_from_store(
    store: &dyn Store,
    name: &RepoName,
) -> Result<ServiceRepository, StoreError> {
    store
        .get_repository(name)
        .await
        .map(|config| ServiceRepository::from((name.clone(), config)))
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
