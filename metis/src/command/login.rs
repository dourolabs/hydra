use anyhow::{anyhow, bail, Context, Result};
use metis_common::users::{CreateUserRequest, UpdateGithubTokenRequest, Username};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::Duration,
};
use tokio::time::{sleep, Instant};

use crate::{client::MetisClientInterface, config};

const AUTH_TOKEN_PATH: &str = "~/.local/share/metis/auth-token";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_PROFILE_URL: &str = "https://api.github.com/user";
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

#[derive(Debug, Deserialize)]
struct GithubUserResponse {
    login: String,
    id: u64,
}

enum TokenPollState {
    Pending,
    SlowDown,
    Token(String),
}

pub async fn run(client: &dyn MetisClientInterface) -> Result<()> {
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
    let profile = fetch_github_profile(&http, &token).await?;
    store_github_credentials(client, &profile, &token).await?;
    let token_path = write_auth_token_file(&token)?;

    println!(
        "Logged in as {}. Stored token at {}.",
        profile.login,
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

async fn fetch_github_profile(http: &reqwest::Client, token: &str) -> Result<GithubUserResponse> {
    let response = http
        .get(USER_PROFILE_URL)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, USER_AGENT_VALUE)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to fetch GitHub user profile")?
        .error_for_status()
        .context("GitHub user profile request failed")?;

    response
        .json::<GithubUserResponse>()
        .await
        .context("failed to decode GitHub user profile response")
}

async fn store_github_credentials(
    client: &dyn MetisClientInterface,
    profile: &GithubUserResponse,
    token: &str,
) -> Result<Username> {
    let username: Username = profile.login.as_str().into();
    let create_request = CreateUserRequest {
        username: username.clone(),
        github_user_id: Some(profile.id),
        github_token: token.to_string(),
    };

    match client.create_user(&create_request).await {
        Ok(response) => Ok(response.user.username),
        Err(create_err) => {
            let update_request = UpdateGithubTokenRequest {
                github_token: token.to_string(),
                github_user_id: Some(profile.id),
            };
            match client
                .set_user_github_token(&username, &update_request)
                .await
            {
                Ok(response) => Ok(response.user.username),
                Err(update_err) => Err(anyhow!(
                    "failed to store GitHub credentials (create error: {create_err}; update error: {update_err})"
                )),
            }
        }
    }
}

fn resolve_auth_token_path() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME is not set; cannot resolve auth token path"))?;
    let raw_path = PathBuf::from(AUTH_TOKEN_PATH);
    let expanded = config::expand_path(&raw_path);
    if expanded.to_string_lossy().starts_with('~') {
        return Ok(PathBuf::from(home).join(".local/share/metis/auth-token"));
    }
    Ok(expanded)
}

fn write_auth_token_file(token: &str) -> Result<PathBuf> {
    let path = resolve_auth_token_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("auth token path missing parent directory"))?;
    fs::create_dir_all(parent).context("failed to create auth token directory")?;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .context("failed to open auth token file for writing")?;
    file.write_all(token.as_bytes())
        .context("failed to write auth token file")?;
    file.flush().context("failed to flush auth token file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .context("failed to set auth token file permissions")?;
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn auth_token_path_and_write_use_home_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let original = env::var_os("HOME");
        let temp = tempdir().expect("tempdir");
        env::set_var("HOME", temp.path());

        let path = resolve_auth_token_path().expect("auth path");
        let expected = temp.path().join(".local/share/metis/auth-token");
        assert_eq!(path, expected);

        let written = write_auth_token_file("token-123").expect("write token");
        assert_eq!(written, expected);
        let contents = fs::read_to_string(&written).expect("read token");
        assert_eq!(contents, "token-123");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&written).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        match original {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }
}
