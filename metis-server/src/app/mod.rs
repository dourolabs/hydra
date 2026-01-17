mod app_state;

use crate::config::{ServiceSection, non_empty};
use metis_common::{
    PatchId, RepoName,
    jobs::{Bundle, BundleSpec},
    merge_queues::MergeQueue,
};
use std::{collections::HashMap, sync::Arc};
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
