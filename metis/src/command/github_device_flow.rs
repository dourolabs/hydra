use anyhow::{anyhow, bail, Context, Result};
use metis_common::{api::v1::login::LoginRequest, users::ResolveUserRequest};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};
use tokio::time::{sleep, Instant};

use crate::client::{MetisClient, MetisClientUnauthenticated};

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const GITHUB_SCOPE: &str = "read:user";
const USER_AGENT_VALUE: &str = "metis-cli";

#[derive(Debug, Deserialize)]
struct DeviceFlowResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

enum TokenPollState {
    Pending,
    SlowDown,
    Token(String),
}

pub async fn login_with_github_device_flow(
    client: &MetisClientUnauthenticated,
    token_path: &Path,
) -> Result<MetisClient> {
    let client_id = fetch_github_client_id(client)
        .await
        .context("failed to fetch GitHub client id from server")?;
    let http = reqwest::Client::new();
    let device_flow = start_device_flow(&http, &client_id).await?;

    println!(
        "Open {} and enter code: {}",
        device_flow.verification_uri, device_flow.user_code
    );
    println!("Waiting for authorization...");

    let token = poll_for_token(&http, &client_id, &device_flow).await?;
    let auth_token = exchange_and_store_token(client, token_path, &token).await?;
    let auth_client = MetisClient::new(client.base_url().as_str(), auth_token.clone())
        .context("failed to create authenticated client")?;
    let resolved_user = auth_client
        .resolve_user(&ResolveUserRequest::new(auth_token))
        .await
        .context("failed to resolve user from auth token")?;

    println!(
        "Logged in as {}. Stored token at {}.",
        resolved_user.user.username,
        token_path.display()
    );

    Ok(auth_client)
}

async fn fetch_github_client_id(client: &MetisClientUnauthenticated) -> Result<String> {
    let response = client
        .get_github_app_client_id()
        .await
        .context("github client id request failed")?;
    Ok(response.client_id)
}

async fn start_device_flow(http: &reqwest::Client, client_id: &str) -> Result<DeviceFlowResponse> {
    let response = http
        .post(DEVICE_CODE_URL)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, USER_AGENT_VALUE)
        .form(&[("client_id", client_id), ("scope", GITHUB_SCOPE)])
        .send()
        .await
        .context("failed to contact GitHub device flow endpoint")?
        .error_for_status()
        .context("GitHub device flow returned an error status")?;

    response
        .json::<DeviceFlowResponse>()
        .await
        .context("failed to decode GitHub device flow response")
}

async fn poll_for_token(
    http: &reqwest::Client,
    client_id: &str,
    device_flow: &DeviceFlowResponse,
) -> Result<String> {
    let expires_at = Instant::now() + Duration::from_secs(device_flow.expires_in);
    let mut interval = Duration::from_secs(device_flow.interval.max(1));

    loop {
        if Instant::now() >= expires_at {
            bail!("GitHub device flow code expired. Run `metis login` again.");
        }

        let response = http
            .post(ACCESS_TOKEN_URL)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, USER_AGENT_VALUE)
            .form(&[
                ("client_id", client_id),
                ("device_code", device_flow.device_code.as_str()),
                ("grant_type", DEVICE_GRANT_TYPE),
            ])
            .send()
            .await
            .context("failed to poll GitHub device flow token")?
            .error_for_status()
            .context("GitHub token endpoint returned an error status")?;

        let payload = response
            .json::<TokenPollResponse>()
            .await
            .context("failed to decode GitHub token response")?;

        match interpret_token_response(payload)? {
            TokenPollState::Token(token) => return Ok(token),
            TokenPollState::Pending => {
                sleep(interval).await;
            }
            TokenPollState::SlowDown => {
                interval += Duration::from_secs(5);
                sleep(interval).await;
            }
        }
    }
}

fn interpret_token_response(payload: TokenPollResponse) -> Result<TokenPollState> {
    if let Some(token) = payload.access_token {
        return Ok(TokenPollState::Token(token));
    }

    match payload.error.as_deref() {
        Some("authorization_pending") => Ok(TokenPollState::Pending),
        Some("slow_down") => Ok(TokenPollState::SlowDown),
        Some("expired_token") => bail!("GitHub device flow expired. Run `metis login` again."),
        Some("access_denied") => bail!("GitHub device flow denied authorization."),
        Some(other) => {
            let description = payload
                .error_description
                .unwrap_or_else(|| "Unknown GitHub device flow error".to_string());
            bail!("GitHub device flow error ({other}): {description}")
        }
        None => Err(anyhow!(
            "GitHub device flow response did not include an access token"
        )),
    }
}

async fn login_with_github_token(
    client: &MetisClientUnauthenticated,
    token: &str,
) -> Result<String> {
    let request = LoginRequest::new(token.to_string());
    let (auth_token, _client) = client
        .login(&request)
        .await
        .context("failed to exchange GitHub token for Metis login token")?;
    Ok(auth_token)
}

async fn exchange_and_store_token(
    client: &MetisClientUnauthenticated,
    token_path: &Path,
    github_token: &str,
) -> Result<String> {
    let auth_token = login_with_github_token(client, github_token).await?;
    write_auth_token_file(token_path, &auth_token)?;
    Ok(auth_token)
}

fn write_auth_token_file(token_path: &Path, token: &str) -> Result<()> {
    let parent = token_path
        .parent()
        .ok_or_else(|| anyhow!("auth token path missing parent directory"))?;
    fs::create_dir_all(parent).context("failed to create auth token directory")?;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(token_path)
        .context("failed to open auth token file for writing")?;
    file.write_all(token.as_bytes())
        .context("failed to write auth token file")?;
    file.flush().context("failed to flush auth token file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(token_path, fs::Permissions::from_mode(0o600))
            .context("failed to set auth token file permissions")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClientUnauthenticated;
    use httpmock::prelude::*;
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn interpret_token_response_handles_pending_states() {
        let pending = interpret_token_response(TokenPollResponse {
            access_token: None,
            error: Some("authorization_pending".to_string()),
            error_description: None,
        })
        .unwrap();
        assert!(matches!(pending, TokenPollState::Pending));

        let slow_down = interpret_token_response(TokenPollResponse {
            access_token: None,
            error: Some("slow_down".to_string()),
            error_description: None,
        })
        .unwrap();
        assert!(matches!(slow_down, TokenPollState::SlowDown));
    }

    #[test]
    fn auth_token_path_and_write_use_provided_path() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");

        write_auth_token_file(&token_path, "token-123").expect("write token");
        let contents = fs::read_to_string(&token_path).expect("read token");
        assert_eq!(contents, "token-123");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&token_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[tokio::test]
    async fn exchange_and_store_token_uses_login_response_token() {
        let server = MockServer::start();
        let login_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/login")
                .json_body(json!({ "github_token": "gh-token" }));
            then.status(200).json_body(json!({
                "login_token": "api-token",
                "user": {
                    "username": "octo",
                    "github_user_id": null
                }
            }));
        });

        let client =
            MetisClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())
                .expect("client");
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");

        let auth_token = exchange_and_store_token(&client, &token_path, "gh-token")
            .await
            .expect("exchange");

        login_mock.assert();
        let contents = fs::read_to_string(&token_path).expect("read token");
        assert_eq!(contents, "api-token");
        assert_eq!(auth_token, "api-token");
    }
}
