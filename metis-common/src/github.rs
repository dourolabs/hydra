use anyhow::{Context, Result, anyhow, bail};
use octocrab::Octocrab;
use serde::Deserialize;

pub const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";
pub const DEFAULT_GITHUB_PER_PAGE: u8 = 100;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct GithubConfig {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub per_page: Option<u8>,
}

#[derive(Debug)]
pub struct GithubClient {
    client: Octocrab,
    per_page: u8,
    api_base_url: String,
}

impl GithubConfig {
    pub fn build_client(&self) -> Result<GithubClient> {
        self.build_client_with_token(self.token.clone())
    }

    pub fn build_client_with_token(&self, token_override: Option<String>) -> Result<GithubClient> {
        let per_page = self.per_page.unwrap_or(DEFAULT_GITHUB_PER_PAGE);
        validate_per_page(per_page)?;

        let api_base_url = self
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_GITHUB_API_URL)
            .to_string();

        let mut builder = Octocrab::builder();
        builder = builder
            .base_uri(api_base_url.clone())
            .with_context(|| format!("invalid GitHub API base url '{api_base_url}'"))?;

        if let Some(token) = token_override.or_else(|| self.token.clone()) {
            let token = token.trim();
            if token.is_empty() {
                bail!("GitHub token may not be empty");
            }
            builder = builder.personal_token(token.to_string());
        }

        let client = builder.build().context("building GitHub client")?;

        Ok(GithubClient {
            client,
            per_page,
            api_base_url,
        })
    }
}

impl GithubClient {
    pub fn client(&self) -> &Octocrab {
        &self.client
    }

    pub fn into_client(self) -> Octocrab {
        self.client
    }

    pub fn per_page(&self) -> u8 {
        self.per_page
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }
}

fn validate_per_page(value: u8) -> Result<()> {
    if value == 0 {
        Err(anyhow!("GitHub per_page value must be greater than zero"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn builds_client_with_config_values() {
        let config = GithubConfig {
            token: Some("abc123".to_string()),
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
            per_page: Some(50),
        };

        let client = config.build_client().expect("client builds");
        assert_eq!(client.api_base_url(), "https://ghe.example.com/api/v3");
        assert_eq!(client.per_page(), 50);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn defaults_to_public_api_and_max_per_page() {
        let config = GithubConfig::default();
        let client = config.build_client().expect("client builds");

        assert_eq!(client.api_base_url(), DEFAULT_GITHUB_API_URL);
        assert_eq!(client.per_page(), DEFAULT_GITHUB_PER_PAGE);
    }

    #[test]
    fn rejects_zero_per_page() {
        let config = GithubConfig {
            token: None,
            api_base_url: None,
            per_page: Some(0),
        };

        let err = config.build_client().expect_err("per_page must be > 0");
        assert!(
            err.to_string()
                .contains("per_page value must be greater than zero")
        );
    }
}
