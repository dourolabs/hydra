use anyhow::{anyhow, bail, Context, Result};
use metis_common::{api::v1::login::LoginRequest, whoami::ActorIdentity};
use owo_colors::OwoColorize;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::io::IsTerminal;
use std::{path::Path, time::Duration};
use tokio::time::{sleep, Instant};

use crate::{
    client::{MetisClient, MetisClientUnauthenticated},
    config,
};

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
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug)]
struct DeviceFlowToken {
    access_token: String,
    refresh_token: String,
}

enum TokenPollState {
    Pending,
    SlowDown,
    Token(DeviceFlowToken),
}

pub async fn login_with_github_device_flow(
    client: &MetisClientUnauthenticated,
    config_path: &Path,
    server_url: &str,
) -> Result<MetisClient> {
    let client_id = fetch_github_client_id(client)
        .await
        .context("failed to fetch GitHub client id from server")?;
    let http = reqwest::Client::new();
    let device_flow = start_device_flow(&http, &client_id).await?;

    print_login_banner();
    print_login_instructions(&device_flow.verification_uri, &device_flow.user_code);

    let token = poll_for_token(&http, &client_id, &device_flow).await?;
    let auth_token = exchange_and_store_token(client, config_path, server_url, &token).await?;
    let auth_client = MetisClient::new(client.base_url().as_str(), auth_token.clone())
        .context("failed to create authenticated client")?;
    let whoami = auth_client
        .whoami()
        .await
        .context("failed to fetch authenticated user")?;
    let actor_label = match whoami.actor {
        ActorIdentity::User { username } => username.to_string(),
        ActorIdentity::Session { session_id, .. } => session_id.to_string(),
        _ => "unknown".to_string(),
    };

    println!(
        "Logged in as {}. Stored token in {}.",
        actor_label,
        config_path.display()
    );

    Ok(auth_client)
}

fn print_login_banner() {
    for line in login_banner_lines() {
        println!("{line}");
    }
    println!();
}

fn print_login_instructions(verification_uri: &str, user_code: &str) {
    let color_enabled = supports_color();
    let url = if color_enabled {
        verification_uri.bright_blue().bold().to_string()
    } else {
        verification_uri.to_string()
    };
    let code = if color_enabled {
        user_code.bright_yellow().bold().to_string()
    } else {
        user_code.to_string()
    };
    println!("Open {url} and enter code {code}");
    if color_enabled {
        println!("{}", "Waiting for authorization...".dimmed());
    } else {
        println!("Waiting for authorization...");
    }
}

fn login_banner_lines() -> Vec<String> {
    let color_enabled = supports_color();
    let lines = [
        r"__/\\\\____________/\\\\__/\\\\\\\\\\\\\\\__/\\\\\\\\\\\\\\\__/\\\\\\\\\\\_____/\\\\\\\\\\\___        ",
        r" _\/\\\\\\________/\\\\\\_\/\\\///////////__\///////\\\/////__\/////\\\///____/\\\/////////\\\_       ",
        r"  _\/\\\//\\\____/\\\//\\\_\/\\\___________________\/\\\___________\/\\\______\//\\\______\///__      ",
        r"   _\/\\\\///\\\/\\\/_\/\\\_\/\\\\\\\\\\\___________\/\\\___________\/\\\_______\////\\\_________     ",
        r"    _\/\\\__\///\\\/___\/\\\_\/\\\///////____________\/\\\___________\/\\\__________\////\\\______    ",
        r"     _\/\\\____\///_____\/\\\_\/\\\___________________\/\\\___________\/\\\_____________\////\\\___   ",
        r"      _\/\\\_____________\/\\\_\/\\\___________________\/\\\___________\/\\\______/\\\______\//\\\__  ",
        r"       _\/\\\_____________\/\\\_\/\\\\\\\\\\\\\\\_______\/\\\________/\\\\\\\\\\\_\///\\\\\\\\\\\/___ ",
        r"        _\///______________\///__\///////////////________\///________\///////////____\///////////_____",
    ];
    lines
        .into_iter()
        .map(|line| colorize_raised(line, color_enabled))
        .collect()
}

fn colorize_raised(line: &str, color_enabled: bool) -> String {
    if !color_enabled {
        return line.to_string();
    }

    line.chars()
        .map(|ch| {
            if ch == '_' || ch == ' ' {
                ch.to_string()
            } else {
                format!("{ch}").bright_cyan().bold().to_string()
            }
        })
        .collect()
}

fn supports_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if matches!(std::env::var_os("TERM").as_deref(), Some(term) if term == "dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
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
) -> Result<DeviceFlowToken> {
    let expires_at = Instant::now() + Duration::from_secs(device_flow.expires_in);
    let mut interval = Duration::from_secs(device_flow.interval.max(1));

    loop {
        if Instant::now() >= expires_at {
            bail!("GitHub device flow code expired. Run `metis users login` again.");
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
        return Ok(TokenPollState::Token(DeviceFlowToken {
            access_token: token,
            refresh_token: payload
                .refresh_token
                .ok_or_else(|| anyhow!("GitHub device flow response missing refresh token"))?,
        }));
    }

    match payload.error.as_deref() {
        Some("authorization_pending") => Ok(TokenPollState::Pending),
        Some("slow_down") => Ok(TokenPollState::SlowDown),
        Some("expired_token") => {
            bail!("GitHub device flow expired. Run `metis users login` again.")
        }
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
    refresh_token: &str,
) -> Result<String> {
    let request = LoginRequest::new(token.to_string(), refresh_token.to_string());
    let (auth_token, _client) = client
        .login(&request)
        .await
        .context("failed to exchange GitHub token for Metis login token")?;
    Ok(auth_token)
}

async fn exchange_and_store_token(
    client: &MetisClientUnauthenticated,
    config_path: &Path,
    server_url: &str,
    github_token: &DeviceFlowToken,
) -> Result<String> {
    let auth_token = login_with_github_token(
        client,
        &github_token.access_token,
        &github_token.refresh_token,
    )
    .await?;
    config::store_auth_token(config_path, server_url, &auth_token)?;
    Ok(auth_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClientUnauthenticated;
    use crate::config::AppConfig;
    use httpmock::prelude::*;
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn interpret_token_response_handles_pending_states() {
        let pending = interpret_token_response(TokenPollResponse {
            access_token: None,
            refresh_token: None,
            error: Some("authorization_pending".to_string()),
            error_description: None,
        })
        .unwrap();
        assert!(matches!(pending, TokenPollState::Pending));

        let slow_down = interpret_token_response(TokenPollResponse {
            access_token: None,
            refresh_token: None,
            error: Some("slow_down".to_string()),
            error_description: None,
        })
        .unwrap();
        assert!(matches!(slow_down, TokenPollState::SlowDown));
    }

    #[tokio::test]
    async fn exchange_and_store_token_uses_login_response_token() {
        let server = MockServer::start();
        let login_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/login").json_body(json!({
                "github_token": "gh-token",
                "github_refresh_token": "refresh-token"
            }));
            then.status(200).json_body(json!({
                "login_token": "api-token",
                "user": {
                    "username": "octo",
                    "github_user_id": 42
                }
            }));
        });

        let client =
            MetisClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())
                .expect("client");
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let server_url = server.base_url();

        let auth_token = exchange_and_store_token(
            &client,
            &config_path,
            &server_url,
            &DeviceFlowToken {
                access_token: "gh-token".to_string(),
                refresh_token: "refresh-token".to_string(),
            },
        )
        .await
        .expect("exchange");

        login_mock.assert();
        assert_eq!(auth_token, "api-token");

        let stored_config = AppConfig::load(&config_path).expect("load config");
        let stored_token = stored_config
            .auth_token_for_url(&server_url)
            .expect("lookup token");
        assert_eq!(stored_token, Some("api-token"));
    }
}
