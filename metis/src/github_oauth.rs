use anyhow::{anyhow, Context, Result};
use reqwest::{header, Client, Url};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::time::{sleep, Duration, Instant};

const DEVICE_CODE_PATH: &str = "/login/device/code";
const ACCESS_TOKEN_PATH: &str = "/login/oauth/access_token";
const USER_PATH: &str = "/user";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AccessTokenResponse {
    Success {
        access_token: String,
        #[allow(dead_code)]
        token_type: Option<String>,
        #[allow(dead_code)]
        scope: Option<String>,
    },
    Error {
        error: String,
        error_description: Option<String>,
        error_uri: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    login: String,
}

#[derive(Clone)]
pub struct GitHubOAuthDeviceFlow {
    http: Client,
    oauth_base_url: Url,
    api_base_url: Url,
}

impl GitHubOAuthDeviceFlow {
    pub fn new(
        http: Client,
        oauth_base_url: impl AsRef<str>,
        api_base_url: impl AsRef<str>,
    ) -> Result<Self> {
        let oauth_url = Url::parse(oauth_base_url.as_ref()).with_context(|| {
            format!(
                "invalid GitHub OAuth base URL '{}'",
                oauth_base_url.as_ref()
            )
        })?;
        let api_url = Url::parse(api_base_url.as_ref())
            .with_context(|| format!("invalid GitHub API base URL '{}'", api_base_url.as_ref()))?;

        Ok(Self {
            http,
            oauth_base_url: oauth_url,
            api_base_url: api_url,
        })
    }

    pub async fn request_device_code(&self, client_id: &str) -> Result<DeviceCodeResponse> {
        let url = self
            .oauth_base_url
            .join(DEVICE_CODE_PATH)
            .context("failed to build GitHub device code URL")?;

        let response = self
            .http
            .post(url)
            .header(header::ACCEPT, "application/json")
            .form(&[("client_id", client_id)])
            .send()
            .await
            .context("failed to request GitHub device code")?
            .error_for_status()
            .context("GitHub device code endpoint returned an error status")?;

        response
            .json::<DeviceCodeResponse>()
            .await
            .context("failed to decode GitHub device code response")
    }

    pub async fn poll_access_token(
        &self,
        client_id: &str,
        device_code: &str,
        expires_in: u64,
        interval: u64,
    ) -> Result<String> {
        self.poll_access_token_with_sleep(
            client_id,
            device_code,
            expires_in,
            interval,
            |duration| sleep(duration),
        )
        .await
    }

    async fn poll_access_token_with_sleep<F, Fut>(
        &self,
        client_id: &str,
        device_code: &str,
        expires_in: u64,
        interval: u64,
        mut sleep_fn: F,
    ) -> Result<String>
    where
        F: FnMut(Duration) -> Fut,
        Fut: Future<Output = ()>,
    {
        let url = self
            .oauth_base_url
            .join(ACCESS_TOKEN_PATH)
            .context("failed to build GitHub access token URL")?;
        let start = Instant::now();
        let mut poll_interval = interval;

        loop {
            if start.elapsed() >= Duration::from_secs(expires_in) {
                return Err(anyhow!("GitHub device code expired"));
            }

            let response = self
                .http
                .post(url.clone())
                .header(header::ACCEPT, "application/json")
                .form(&[
                    ("client_id", client_id),
                    ("device_code", device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await
                .context("failed to request GitHub access token")?;

            if !response.status().is_success() {
                return Err(anyhow!(
                    "GitHub access token endpoint returned status {}",
                    response.status()
                ));
            }

            let payload = response
                .json::<AccessTokenResponse>()
                .await
                .context("failed to decode GitHub access token response")?;

            match payload {
                AccessTokenResponse::Success { access_token, .. } => return Ok(access_token),
                AccessTokenResponse::Error {
                    error,
                    error_description,
                    error_uri,
                } => match error.as_str() {
                    "authorization_pending" => {
                        sleep_fn(Duration::from_secs(poll_interval)).await;
                    }
                    "slow_down" => {
                        poll_interval = poll_interval.saturating_add(5);
                        sleep_fn(Duration::from_secs(poll_interval)).await;
                    }
                    "expired_token" => {
                        return Err(anyhow!("GitHub device code expired"));
                    }
                    "access_denied" => {
                        return Err(anyhow!("GitHub device authorization denied"));
                    }
                    _ => {
                        let mut details = String::new();
                        if let Some(description) = error_description {
                            details.push_str(&format!(": {description}"));
                        } else if let Some(uri) = error_uri {
                            details.push_str(&format!(": {uri}"));
                        }
                        return Err(anyhow!("GitHub device flow failed: {error}{details}"));
                    }
                },
            }
        }
    }

    pub async fn fetch_username(&self, access_token: &str, user_agent: &str) -> Result<String> {
        let url = self
            .api_base_url
            .join(USER_PATH)
            .context("failed to build GitHub user URL")?;

        let response = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json")
            .header(header::USER_AGENT, user_agent)
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .send()
            .await
            .context("failed to request GitHub user profile")?
            .error_for_status()
            .context("GitHub user profile endpoint returned an error status")?;

        let payload = response
            .json::<UserResponse>()
            .await
            .context("failed to decode GitHub user profile response")?;

        Ok(payload.login)
    }

    pub async fn complete_device_flow(
        &self,
        client_id: &str,
        device_response: &DeviceCodeResponse,
        user_agent: &str,
    ) -> Result<(String, String)> {
        let token = self
            .poll_access_token(
                client_id,
                &device_response.device_code,
                device_response.expires_in,
                device_response.interval,
            )
            .await?;
        let username = self.fetch_username(&token, user_agent).await?;

        Ok((username, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{prelude::*, Mock};
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn device_flow_success_returns_username_and_token() -> Result<()> {
        let server = MockServer::start();
        let oauth =
            GitHubOAuthDeviceFlow::new(HttpClient::new(), server.base_url(), server.base_url())?;

        let device_response = DeviceCodeResponse {
            device_code: "device-123".to_string(),
            user_code: "user-123".to_string(),
            verification_uri: "https://example.com/verify".to_string(),
            expires_in: 600,
            interval: 1,
        };

        let device_mock = server.mock(|when, then| {
            when.method(POST)
                .path(DEVICE_CODE_PATH)
                .body_contains("client_id=client-123");
            then.status(200).json_body_obj(&device_response);
        });

        let token_mock = server.mock(|when, then| {
            when.method(POST)
                .path(ACCESS_TOKEN_PATH)
                .body_contains("device_code=device-123");
            then.status(200)
                .json_body_obj(&json!({"access_token": "gh-token"}));
        });

        let user_mock = server.mock(|when, then| {
            when.method(GET)
                .path(USER_PATH)
                .header("authorization", "Bearer gh-token")
                .header("user-agent", "metis-cli");
            then.status(200).json_body_obj(&json!({"login": "octocat"}));
        });

        let device = oauth.request_device_code("client-123").await?;
        assert_eq!(device.device_code, device_response.device_code);

        let (username, token) = oauth
            .complete_device_flow("client-123", &device, "metis-cli")
            .await?;

        assert_eq!(username, "octocat");
        assert_eq!(token, "gh-token");

        device_mock.assert();
        token_mock.assert();
        user_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn poll_access_token_handles_pending_and_slow_down() -> Result<()> {
        let server = MockServer::start();
        let oauth =
            GitHubOAuthDeviceFlow::new(HttpClient::new(), server.base_url(), server.base_url())?;

        let mut pending_mock = server.mock(|when, then| {
            when.method(POST).path(ACCESS_TOKEN_PATH);
            then.status(200)
                .json_body_obj(&json!({"error": "authorization_pending"}));
        });
        let mut slow_mock: Option<Mock> = None;
        let mut phase = 0;

        let sleep_calls = Arc::new(Mutex::new(Vec::new()));
        let sleep_calls_handle = sleep_calls.clone();

        let token = oauth
            .poll_access_token_with_sleep("client-123", "device-123", 600, 1, |duration| {
                sleep_calls_handle
                    .lock()
                    .expect("sleep call lock")
                    .push(duration);

                match phase {
                    0 => {
                        pending_mock.delete();
                        slow_mock = Some(server.mock(|when, then| {
                            when.method(POST).path(ACCESS_TOKEN_PATH);
                            then.status(200)
                                .json_body_obj(&json!({"error": "slow_down"}));
                        }));
                        phase = 1;
                    }
                    1 => {
                        if let Some(ref mut mock) = slow_mock {
                            mock.delete();
                        }
                        let _ = server.mock(|when, then| {
                            when.method(POST).path(ACCESS_TOKEN_PATH);
                            then.status(200)
                                .json_body_obj(&json!({"access_token": "gh-token"}));
                        });
                        phase = 2;
                    }
                    _ => {}
                }

                std::future::ready(())
            })
            .await?;
        assert_eq!(token, "gh-token");

        let durations = sleep_calls.lock().expect("sleep call lock");
        assert_eq!(
            durations.as_slice(),
            [Duration::from_secs(1), Duration::from_secs(6)]
        );

        Ok(())
    }
}
