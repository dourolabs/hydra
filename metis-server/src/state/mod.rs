use crate::config::{ServiceSection, non_empty};
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
}
