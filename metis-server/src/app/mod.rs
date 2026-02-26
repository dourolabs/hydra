mod agents;
mod app_state;
mod documents;
pub mod event_bus;
mod issues;
mod jobs;
mod merge_queue;
mod messages;
mod patches;
mod repositories;
mod resolved_task;
#[cfg(test)]
pub mod test_helpers;
mod users;

use crate::{
    domain::jobs::Bundle, domain::patches::Patch, merge_queue::MergeQueueImpl, store::StoreError,
};
use git2::Repository as GitRepository;
use metis_common::{PatchId, RepoName, merge_queues::MergeQueue};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tempfile::TempDir;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

pub use agents::AgentError;
pub use app_state::AppState;
pub use documents::UpsertDocumentError;
pub use event_bus::{EventBus, ServerEvent, StoreWithEvents};
pub use issues::{UpdateTodoListError, UpsertIssueError};
pub use jobs::{CreateJobError, SetJobStatusError};
pub(crate) use jobs::{WORKER_NAME_CLEANUP_ORPHANED_TASKS, WORKER_NAME_TASK_LIFECYCLE};
pub use messages::SendMessageError;
pub use metis_common::repositories::{Repository, RepositoryRecord};
pub use patches::UpsertPatchError;
pub use resolved_task::{ResolvedTask, TaskResolutionError};
pub use users::LoginError;
pub(crate) use users::WORKER_NAME_LOGIN;

#[derive(Debug, Clone)]
pub struct ResolvedBundle {
    pub bundle: Bundle,
    pub default_image: Option<String>,
}

/// Aggregated cache for repositories the service can interact with.
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
    #[error("repository store error")]
    Store {
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

fn connect_repository(
    _repo_name: &RepoName,
    config: &Repository,
) -> Result<(GitRepository, TempDir), anyhow::Error> {
    let workdir = TempDir::new()?;
    let repository = GitRepository::clone(&config.remote_url, workdir.path())?;

    Ok((repository, workdir))
}

#[allow(clippy::result_large_err)]
impl ServiceState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn ensure_cached(
        &self,
        repo_name: &RepoName,
        config: &Repository,
    ) -> Result<(), MergeQueueError> {
        self.initialize_merge_queue(repo_name).await;
        self.ensure_git_cache(repo_name, config).await?;
        Ok(())
    }

    pub async fn clear_cache(&self, repo_name: &RepoName) {
        {
            let mut merge_queues = self.merge_queues.write().await;
            merge_queues.remove(repo_name);
        }
        let mut git_cache = self.git_cache.write().await;
        git_cache.remove(repo_name);
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

    pub async fn get_merge_queue(
        &self,
        repo_name: &RepoName,
        config: &Repository,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.ensure_cached(repo_name, config).await?;

        let merge_queues = self.merge_queues.read().await;
        let queue = merge_queues
            .get(repo_name)
            .and_then(|repo_queues| repo_queues.get(branch_name))
            .map(merge_queue_response)
            .unwrap_or_default();

        Ok(queue)
    }

    pub async fn add_patch_to_merge_queue(
        &self,
        repo_name: &RepoName,
        config: &Repository,
        branch_name: &str,
        patch_id: PatchId,
        patch: &Patch,
    ) -> Result<MergeQueue, MergeQueueError> {
        self.ensure_cached(repo_name, config).await?;

        if patch.service_repo_name != *repo_name {
            return Err(MergeQueueError::PatchRepositoryMismatch {
                patch_id,
                patch_repo: patch.service_repo_name.clone(),
                service_repo: repo_name.clone(),
            });
        }

        let repository_handle = self.refresh_repository(repo_name, config).await?;

        let mut merge_queues = self.merge_queues.write().await;
        let repo_queues = merge_queues.entry(repo_name.clone()).or_default();
        let branch_name = branch_name.to_string();
        let queue_patch_id = patch_id.clone();
        let queue = match repo_queues.get_mut(&branch_name) {
            Some(queue) => queue,
            None => {
                let queue = MergeQueueImpl::new(&repository_handle, branch_ref(&branch_name))
                    .map_err(|source| MergeQueueError::QueueInitialization {
                        repo_name: repo_name.clone(),
                        branch_name: branch_name.clone(),
                        source,
                    })?;
                repo_queues.insert(branch_name.clone(), queue);
                repo_queues
                    .get_mut(&branch_name)
                    .expect("queue should exist after insertion")
            }
        };
        queue
            .try_squash_append_diff(
                &repository_handle,
                queue_patch_id.clone(),
                &patch.diff,
                Some(&patch.title),
            )
            .map_err(|source| MergeQueueError::QueueUpdate {
                patch_id: queue_patch_id,
                repo_name: repo_name.clone(),
                branch_name: branch_name.clone(),
                source,
            })?;

        Ok(merge_queue_response(queue))
    }

    async fn refresh_repository(
        &self,
        repo_name: &RepoName,
        config: &Repository,
    ) -> Result<GitRepository, MergeQueueError> {
        let cache_entry = self.ensure_git_cache(repo_name, config).await?;

        let cached = cache_entry.lock().await;
        let repository_handle =
            GitRepository::open(&cached.path).map_err(|source| MergeQueueError::Git {
                repo_name: repo_name.clone(),
                source: source.into(),
            })?;
        drop(cached);

        let mut remote =
            repository_handle
                .find_remote("origin")
                .map_err(|source| MergeQueueError::Git {
                    repo_name: repo_name.clone(),
                    source: source.into(),
                })?;
        remote
            .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
            .map_err(|source| MergeQueueError::Git {
                repo_name: repo_name.clone(),
                source: source.into(),
            })?;

        drop(remote);
        Ok(repository_handle)
    }

    async fn ensure_git_cache(
        &self,
        repo_name: &RepoName,
        config: &Repository,
    ) -> Result<Arc<Mutex<CachedRepository>>, MergeQueueError> {
        let mut git_cache = self.git_cache.write().await;
        if let Some(existing) = git_cache.get(repo_name) {
            return Ok(existing.clone());
        }

        let (_repository, _workdir) =
            connect_repository(repo_name, config).map_err(|source| MergeQueueError::Git {
                repo_name: repo_name.clone(),
                source,
            })?;
        let cached = Arc::new(Mutex::new(CachedRepository {
            path: _workdir.path().to_path_buf(),
            _workdir,
        }));
        git_cache.insert(repo_name.clone(), cached.clone());
        Ok(cached)
    }

    async fn initialize_merge_queue(&self, name: &RepoName) {
        let mut merge_queues = self.merge_queues.write().await;
        merge_queues.entry(name.clone()).or_default();
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
    use super::{Repository, connect_repository};
    use anyhow::Result;
    use git2::{Commit, Oid, Repository as GitRepository, Signature};
    use metis_common::RepoName;
    use std::{fs, path::Path, str::FromStr};
    use tempfile::TempDir;

    #[test]
    fn connect_returns_git2_repository_for_remote_url() -> Result<()> {
        let remote_dir = TempDir::new()?;
        let remote_repo = GitRepository::init(remote_dir.path())?;
        let expected_head = commit_file(&remote_repo, "README.md", "hello", "init")?;

        let repo_name = RepoName::from_str("dourolabs/metis")?;
        let repository = Repository::new(
            remote_dir.path().to_str().unwrap().to_string(),
            None,
            None,
            None,
        );

        let (repo, _workdir) = connect_repository(&repo_name, &repository)?;

        assert_eq!(repo.head()?.target(), Some(expected_head));
        let origin = repo.find_remote("origin")?;
        assert_eq!(origin.url(), Some(remote_dir.path().to_str().unwrap()));

        Ok(())
    }

    fn commit_file(repo: &GitRepository, name: &str, contents: &str, message: &str) -> Result<Oid> {
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
