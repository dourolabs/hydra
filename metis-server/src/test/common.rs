use crate::app::ServiceRepository;
use metis_common::{RepoName, TaskId};
use std::str::FromStr;

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
        Some("token-123".to_string()),
        Some("ghcr.io/example/repo:main".to_string()),
    );

    (name, repository)
}
