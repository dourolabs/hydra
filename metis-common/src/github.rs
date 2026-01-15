use crate::constants::{
    ENV_GH_HOST, ENV_GH_TOKEN, ENV_GITHUB_API_URL, ENV_GITHUB_PER_PAGE, ENV_GITHUB_TOKEN,
};
use anyhow::{Context, Result, anyhow};
use octocrab::Octocrab;
use serde::Deserialize;
use std::env;

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
    pub fn from_env() -> Self {
        Self {
            token: env_token(),
            api_base_url: env_api_base_url(),
            per_page: env_per_page().ok().flatten(),
        }
    }

    pub fn resolved_token(&self, token_override: Option<String>) -> Option<String> {
        resolve_token(token_override, self)
    }

    pub fn build_client(&self) -> Result<GithubClient> {
        self.build_client_with_token(None)
    }

    pub fn build_client_with_token(&self, token_override: Option<String>) -> Result<GithubClient> {
        let token = resolve_token(token_override, self);
        let api_base_url = resolve_api_base_url(self)?;
        let per_page = resolve_per_page(self)?;

        let mut builder = Octocrab::builder();
        builder = builder.base_uri(api_base_url.clone())?;
        if let Some(token) = token {
            builder = builder.personal_token(token);
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

fn resolve_token(token_override: Option<String>, config: &GithubConfig) -> Option<String> {
    token_override
        .and_then(non_empty)
        .or_else(|| config.token.clone().and_then(non_empty))
        .or_else(env_token)
}

fn resolve_api_base_url(config: &GithubConfig) -> Result<String> {
    if let Some(url) = config.api_base_url.clone().and_then(non_empty) {
        return Ok(url);
    }
    if let Some(url) = env_api_base_url() {
        return Ok(url);
    }

    Ok(DEFAULT_GITHUB_API_URL.to_string())
}

fn resolve_per_page(config: &GithubConfig) -> Result<u8> {
    if let Some(value) = config.per_page {
        return validate_per_page(value);
    }
    if let Some(value) = env_per_page()? {
        return Ok(value);
    }

    Ok(DEFAULT_GITHUB_PER_PAGE)
}

fn env_token() -> Option<String> {
    env_var(ENV_GH_TOKEN).or_else(|| env_var(ENV_GITHUB_TOKEN))
}

fn env_api_base_url() -> Option<String> {
    if let Some(url) = env_var(ENV_GITHUB_API_URL) {
        return Some(url);
    }
    let host = env_var(ENV_GH_HOST)?;
    let cleaned = host.trim_end_matches('/');
    let with_scheme = if cleaned.starts_with("http://") || cleaned.starts_with("https://") {
        cleaned.to_string()
    } else {
        format!("https://{cleaned}")
    };
    if with_scheme.ends_with("/api/v3") {
        Some(with_scheme)
    } else {
        Some(format!("{with_scheme}/api/v3"))
    }
}

fn env_per_page() -> Result<Option<u8>> {
    let raw = match env::var(ENV_GITHUB_PER_PAGE) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed: u8 = trimmed
        .parse()
        .with_context(|| format!("invalid {ENV_GITHUB_PER_PAGE} value '{trimmed}'"))?;
    validate_per_page(parsed).map(Some)
}

fn validate_per_page(value: u8) -> Result<u8> {
    if value == 0 {
        Err(anyhow!("GitHub per_page value must be greater than zero"))
    } else {
        Ok(value)
    }
}

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().and_then(non_empty)
}

fn non_empty(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    enum PreviousValue {
        Present(String),
        Absent,
    }

    struct EnvGuard {
        key: &'static str,
        previous: PreviousValue,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = match env::var(key) {
                Ok(current) => PreviousValue::Present(current),
                Err(_) => PreviousValue::Absent,
            };
            // Safe because tests serialize environment access with a global mutex.
            unsafe { env::set_var(key, value) };
            Self { key, previous }
        }

        fn clear(key: &'static str) -> Self {
            let previous = match env::var(key) {
                Ok(current) => PreviousValue::Present(current),
                Err(_) => PreviousValue::Absent,
            };
            // Safe because tests serialize environment access with a global mutex.
            unsafe { env::remove_var(key) };
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                PreviousValue::Present(value) => unsafe { env::set_var(self.key, value) },
                PreviousValue::Absent => unsafe { env::remove_var(self.key) },
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn builds_client_using_overrides_and_env() {
        let _lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let _token = EnvGuard::set(ENV_GH_TOKEN, "env-token");
        let _host = EnvGuard::set(ENV_GH_HOST, "ghe.example.com");
        let _per_page = EnvGuard::set(ENV_GITHUB_PER_PAGE, "42");

        let config = GithubConfig {
            token: Some("config-token".to_string()),
            api_base_url: None,
            per_page: Some(88),
        };

        let client = config
            .build_client_with_token(Some("override-token".to_string()))
            .expect("client builds");
        assert_eq!(client.per_page(), 88);
        assert_eq!(client.api_base_url(), "https://ghe.example.com/api/v3");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn uses_defaults_when_no_config_or_env_present() {
        let _lock = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let _token = EnvGuard::clear(ENV_GH_TOKEN);
        let _github_token = EnvGuard::clear(ENV_GITHUB_TOKEN);
        let _host = EnvGuard::clear(ENV_GH_HOST);
        let _api = EnvGuard::clear(ENV_GITHUB_API_URL);
        let _per_page = EnvGuard::clear(ENV_GITHUB_PER_PAGE);

        let client = GithubConfig::default()
            .build_client()
            .expect("client builds");
        assert_eq!(client.per_page(), DEFAULT_GITHUB_PER_PAGE);
        assert_eq!(client.api_base_url(), DEFAULT_GITHUB_API_URL);
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
