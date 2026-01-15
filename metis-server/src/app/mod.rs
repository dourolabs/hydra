mod app_state;

use crate::config::{ServiceSection, non_empty};
use metis_common::jobs::{Bundle, BundleSpec};
use std::collections::HashMap;
use thiserror::Error;

pub use app_state::{AppState, CreateJobError, SetJobStatusError, UpsertPatchError};

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
    pub repositories: HashMap<String, GitRepository>,
}

#[derive(Debug, Error)]
pub enum BundleResolutionError {
    #[error("unknown repository '{0}'")]
    UnknownRepository(String),
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

        Self { repositories }
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
}
