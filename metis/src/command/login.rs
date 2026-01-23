use anyhow::{anyhow, bail, Context, Result};
use metis_common::api::v1::login::{LoginRequest, LoginResponse};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::Duration,
};
use tokio::time::{sleep, Instant};

use crate::client::MetisClientInterface;

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

pub async fn run(client: &dyn MetisClientInterface, token_path: &PathBuf) -> Result<()> {
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
    let login_response = login_with_github_token(client, &token).await?;
    write_auth_token_file(token_path, &login_response.login_token)?;

    println!(
        "Logged in as {}. Stored token at {}.",
        login_response.user.username,
        token_path.display()
    );

    Ok(())
}

async fn fetch_github_client_id(client: &dyn MetisClientInterface) -> Result<String> {
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
    client: &dyn MetisClientInterface,
    token: &str,
) -> Result<LoginResponse> {
    let request = LoginRequest::new(token.to_string());
    client
        .login(&request)
        .await
        .context("failed to exchange GitHub token for Metis login token")
}

fn write_auth_token_file(token_path: &PathBuf, token: &str) -> Result<()> {
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
}
