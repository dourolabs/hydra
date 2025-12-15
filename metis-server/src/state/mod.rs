use crate::config::{ServiceSection, non_empty};
use crate::routes::jobs::ApiError;
use metis_common::jobs::{Bundle, BundleSpec};
use std::collections::HashMap;

/// Connection details for a git repository remote.
#[derive(Debug, Clone)]
pub struct GitRepository {
    pub name: String,
    pub remote_url: String,
    pub default_branch: Option<String>,
    pub github_token: Option<String>,
}

/// Aggregated state for repositories the service can interact with.
#[derive(Debug, Default, Clone)]
pub struct ServiceState {
    pub repositories: HashMap<String, GitRepository>,
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
                        name: name.clone(),
                        remote_url: repo.remote_url.clone(),
                        default_branch,
                        github_token,
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
    ) -> Result<(Bundle, Option<String>), ApiError> {
        match spec {
            BundleSpec::None => Ok((Bundle::None, None)),
            BundleSpec::TarGz { archive_base64 } => Ok((Bundle::TarGz { archive_base64 }, None)),
            BundleSpec::GitRepository { url, rev } => {
                Ok((Bundle::GitRepository { url, rev }, None))
            }
            BundleSpec::GitBundle { bundle_base64 } => {
                Ok((Bundle::GitBundle { bundle_base64 }, None))
            }
            BundleSpec::ServiceRepository { name, rev } => {
                let repo = self
                    .repositories
                    .get(&name)
                    .ok_or_else(|| ApiError::bad_request(format!("unknown repository '{name}'")))?;

                let resolved_rev = rev
                    .or_else(|| repo.default_branch.clone())
                    .unwrap_or_else(|| "main".to_string());

                Ok((
                    Bundle::GitRepository {
                        url: repo.remote_url.clone(),
                        rev: resolved_rev,
                    },
                    repo.github_token.clone(),
                ))
            }
        }
    }
}
