use crate::app::GitRepository;
use metis_common::{
    RepoName, TaskId,
    patches::{GitOid, PatchCommitRange},
};
use std::str::FromStr;

pub(crate) fn default_image() -> String {
    crate::config::MetisSection::default().worker_image
}

pub(crate) fn task_id(value: &str) -> TaskId {
    value.parse().expect("task id should be valid")
}

pub(crate) fn service_repo_name() -> RepoName {
    RepoName::from_str("dourolabs/private-repo").expect("service repo name should parse")
}

pub(crate) fn patch_commit_range() -> PatchCommitRange {
    PatchCommitRange {
        base: GitOid::from_str("0000000000000000000000000000000000000001").unwrap(),
        head: GitOid::from_str("0000000000000000000000000000000000000002").unwrap(),
    }
}

pub(crate) fn service_repository() -> (RepoName, GitRepository) {
    let name = service_repo_name();
    let repository = GitRepository {
        remote_url: format!("https://example.com/{}.git", name.as_str()),
        default_branch: Some("develop".to_string()),
        github_token: Some("token-123".to_string()),
        default_image: Some("ghcr.io/example/repo:main".to_string()),
    };

    (name, repository)
}
