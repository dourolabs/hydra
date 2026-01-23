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
pub struct ServiceRepository {
    pub name: RepoName,
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

impl ServiceRepository {
    pub fn without_secret(&self) -> ServiceRepositoryInfo {
        ServiceRepositoryInfo {
            name: self.name.clone(),
            remote_url: self.remote_url.clone(),
            default_branch: self.default_branch.clone(),
            default_image: self.default_image.clone(),
        }
    }

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

impl From<(RepoName, ServiceRepositoryConfig)> for ServiceRepository {
    fn from((name, config): (RepoName, ServiceRepositoryConfig)) -> Self {
        Self {
            name,
            remote_url: config.remote_url,
            default_branch: config.default_branch,
            default_image: config.default_image,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn converts_to_info_without_secret_fields() {
        let repo = ServiceRepository {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/repo.git".to_string(),
            default_branch: Some("main".to_string()),
            default_image: Some("image".to_string()),
        };

        let info: ServiceRepositoryInfo = repo.without_secret();
        assert_eq!(info.name.as_str(), "dourolabs/metis");
        assert_eq!(info.default_branch.as_deref(), Some("main"));
        assert_eq!(info.default_image.as_deref(), Some("image"));
    }
}
