use crate::RepoName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServiceRepositoryConfig {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

impl ServiceRepositoryConfig {
    pub fn new(
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            remote_url,
            default_branch,
            default_image,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServiceRepositoryInfo {
    pub name: RepoName,
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

impl ServiceRepositoryInfo {
    pub fn new(
        name: RepoName,
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            name,
            remote_url,
            default_branch,
            default_image,
        }
    }
}

impl From<(RepoName, ServiceRepositoryConfig)> for ServiceRepositoryInfo {
    fn from((name, config): (RepoName, ServiceRepositoryConfig)) -> Self {
        Self::new(
            name,
            config.remote_url,
            config.default_branch,
            config.default_image,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateRepositoryRequest {
    pub name: RepoName,
    #[serde(flatten)]
    pub repository: ServiceRepositoryConfig,
}

impl CreateRepositoryRequest {
    pub fn new(name: RepoName, repository: ServiceRepositoryConfig) -> Self {
        Self { name, repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpdateRepositoryRequest {
    #[serde(flatten)]
    pub repository: ServiceRepositoryConfig,
}

impl UpdateRepositoryRequest {
    pub fn new(repository: ServiceRepositoryConfig) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertRepositoryResponse {
    pub repository: ServiceRepositoryInfo,
}

impl UpsertRepositoryResponse {
    pub fn new(repository: ServiceRepositoryInfo) -> Self {
        Self { repository }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<ServiceRepositoryInfo>,
}

impl ListRepositoriesResponse {
    pub fn new(repositories: Vec<ServiceRepositoryInfo>) -> Self {
        Self { repositories }
    }
}
