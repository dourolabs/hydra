use crate::RepoName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRepositoryConfig {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRepository {
    pub name: RepoName,
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

impl ServiceRepository {
    pub fn github_token_present(&self) -> bool {
        token_present(&self.github_token)
    }

    pub fn without_secret(&self) -> ServiceRepositoryInfo {
        ServiceRepositoryInfo {
            name: self.name.clone(),
            remote_url: self.remote_url.clone(),
            default_branch: self.default_branch.clone(),
            default_image: self.default_image.clone(),
            github_token_present: self.github_token_present(),
        }
    }
}

impl From<(RepoName, ServiceRepositoryConfig)> for ServiceRepository {
    fn from((name, config): (RepoName, ServiceRepositoryConfig)) -> Self {
        Self {
            name,
            remote_url: config.remote_url,
            default_branch: config.default_branch,
            github_token: config.github_token,
            default_image: config.default_image,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRepositoryInfo {
    pub name: RepoName,
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
    #[serde(default)]
    pub github_token_present: bool,
}

fn token_present(token: &Option<String>) -> bool {
    token
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRepositoryRequest {
    pub name: RepoName,
    #[serde(flatten)]
    pub repository: ServiceRepositoryConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRepositoryRequest {
    #[serde(flatten)]
    pub repository: ServiceRepositoryConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertRepositoryResponse {
    pub repository: ServiceRepositoryInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<ServiceRepositoryInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn converts_to_info_with_token_presence() {
        let repo = ServiceRepository {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/repo.git".to_string(),
            default_branch: Some("main".to_string()),
            github_token: Some("token".to_string()),
            default_image: Some("image".to_string()),
        };

        let info: ServiceRepositoryInfo = repo.without_secret();
        assert!(info.github_token_present);
        assert_eq!(info.name.as_str(), "dourolabs/metis");
    }

    #[test]
    fn reports_absent_token() {
        let repo = ServiceRepository {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/repo.git".to_string(),
            default_branch: Some("main".to_string()),
            github_token: Some("   ".to_string()),
            default_image: None,
        };

        let info: ServiceRepositoryInfo = repo.without_secret();
        assert!(!info.github_token_present);
    }
}
