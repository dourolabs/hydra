use crate::RepoName;
use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Repository {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
}

impl Repository {
    pub fn new(
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            remote_url,
            default_branch,
            default_image,
            deleted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RepositoryRecord {
    pub name: RepoName,
    pub repository: Repository,
}

impl RepositoryRecord {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

impl From<(RepoName, Repository)> for RepositoryRecord {
    fn from((name, repository): (RepoName, Repository)) -> Self {
        Self::new(name, repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateRepositoryRequest {
    pub name: RepoName,
    #[serde(flatten)]
    pub repository: Repository,
}

impl CreateRepositoryRequest {
    pub fn new(name: RepoName, repository: Repository) -> Self {
        Self { name, repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpdateRepositoryRequest {
    #[serde(flatten)]
    pub repository: Repository,
}

impl UpdateRepositoryRequest {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl UpsertRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchRepositoriesQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

impl SearchRepositoriesQuery {
    pub fn new(include_deleted: Option<bool>) -> Self {
        Self { include_deleted }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<RepositoryRecord>,
}

impl ListRepositoriesResponse {
    pub fn new(repositories: Vec<RepositoryRecord>) -> Self {
        Self { repositories }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeleteRepositoryResponse {
    pub repository: RepositoryRecord,
}

impl DeleteRepositoryResponse {
    pub fn new(repository: RepositoryRecord) -> Self {
        Self { repository }
    }
}
