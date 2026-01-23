use crate::app::{AppState, ServiceRepository, ServiceRepositoryConfig, ServiceState};
use anyhow::Result;
use metis_common::{RepoName, TaskId};
use std::str::FromStr;
use std::sync::Arc;

pub(crate) fn default_image() -> String {
    "metis-worker:latest".to_string()
}

pub(crate) fn task_id(value: &str) -> TaskId {
    value.parse().expect("task id should be valid")
}

pub(crate) fn service_repo_name() -> RepoName {
    RepoName::from_str("dourolabs/private-repo").expect("service repo name should parse")
}

pub(crate) fn patch_diff() -> String {
    "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
}

pub(crate) fn service_repository() -> (RepoName, ServiceRepository) {
    let name = service_repo_name();
    let repository = ServiceRepository::new(
        name.clone(),
        format!("https://example.com/{}.git", name.as_str()),
        Some("develop".to_string()),
        Some("ghcr.io/example/repo:main".to_string()),
    );

    (name, repository)
}

pub(crate) fn repository_config(repository: &ServiceRepository) -> ServiceRepositoryConfig {
    ServiceRepositoryConfig::new(
        repository.remote_url.clone(),
        repository.default_branch.clone(),
        repository.default_image.clone(),
    )
}

pub(crate) async fn seed_repository(
    state: &mut AppState,
    repository: ServiceRepository,
) -> Result<()> {
    let name = repository.name.clone();
    {
        let mut store = state.store.write().await;
        store
            .add_repository(name.clone(), repository_config(&repository))
            .await?;
    }
    let repository_names = {
        let store = state.store.read().await;
        store
            .list_repositories()
            .await?
            .into_iter()
            .map(|(repo_name, _)| repo_name)
            .collect::<Vec<_>>()
    };
    state.service_state = Arc::new(ServiceState::with_repository_names(repository_names));
    Ok(())
}
