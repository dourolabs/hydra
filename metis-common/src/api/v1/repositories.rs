use crate::RepoName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GithubAppInstallationConfig {
    pub app_id: u64,
    pub installation_id: u64,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
}

impl GithubAppInstallationConfig {
    pub fn new(
        app_id: u64,
        installation_id: u64,
        private_key: Option<String>,
        key_path: Option<String>,
    ) -> Self {
        Self {
            app_id,
            installation_id,
            private_key,
            key_path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServiceRepositoryConfig {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub github_app: Option<GithubAppInstallationConfig>,
    #[serde(default)]
    pub default_image: Option<String>,
}

impl ServiceRepositoryConfig {
    pub fn new(
        remote_url: String,
        default_branch: Option<String>,
        github_token: Option<String>,
        github_app: Option<GithubAppInstallationConfig>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            remote_url,
            default_branch,
            github_token,
            github_app,
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
    pub github_token: Option<String>,
    #[serde(default)]
    pub github_app: Option<GithubAppInstallationConfig>,
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

    pub fn new(
        name: RepoName,
        remote_url: String,
        default_branch: Option<String>,
        github_token: Option<String>,
        github_app: Option<GithubAppInstallationConfig>,
        default_image: Option<String>,
    ) -> Self {
        Self {
            name,
            remote_url,
            default_branch,
            github_token,
            github_app,
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
            github_token: config.github_token,
            github_app: config.github_app,
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
    #[serde(default)]
    pub github_token_present: bool,
}

impl ServiceRepositoryInfo {
    pub fn new(
        name: RepoName,
        remote_url: String,
        default_branch: Option<String>,
        default_image: Option<String>,
        github_token_present: bool,
    ) -> Self {
        Self {
            name,
            remote_url,
            default_branch,
            default_image,
            github_token_present,
        }
    }
}

fn token_present(token: &Option<String>) -> bool {
    token
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
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
pub struct RepositoryAccessTokenResponse {
    pub token: String,
}

impl RepositoryAccessTokenResponse {
    pub fn new(token: String) -> Self {
        Self { token }
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
    fn converts_to_info_with_token_presence() {
        let repo = ServiceRepository {
            name: RepoName::from_str("dourolabs/metis").unwrap(),
            remote_url: "https://example.com/repo.git".to_string(),
            default_branch: Some("main".to_string()),
            github_token: Some("token".to_string()),
            github_app: None,
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
            github_app: None,
            default_image: None,
        };

        let info: ServiceRepositoryInfo = repo.without_secret();
        assert!(!info.github_token_present);
    }
}
