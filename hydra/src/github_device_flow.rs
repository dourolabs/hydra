use anyhow::{bail, Context, Result};
use hydra_common::api::v1::login::DevicePollStatus;
use hydra_common::whoami::ActorIdentity;
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;
use tokio::time::{sleep, Instant};

use crate::{
    client::{HydraClient, HydraClientUnauthenticated},
    config,
};

pub async fn login_with_github_device_flow(
    client: &HydraClientUnauthenticated,
    config_path: &Path,
    server_url: &str,
) -> Result<HydraClient> {
    let device_flow = client
        .device_start()
        .await
        .context("failed to start device flow via server")?;

    print_login_banner();
    print_login_instructions(&device_flow.verification_uri, &device_flow.user_code);

    let expires_at = Instant::now() + Duration::from_secs(device_flow.expires_in as u64);
    let interval = Duration::from_secs((device_flow.interval as u64).max(1));

    let login_token = loop {
        if Instant::now() >= expires_at {
            bail!("Device flow code expired. Run `hydra login` again.");
        }

        sleep(interval).await;

        let poll_response = client
            .device_poll(&device_flow.device_session_id)
            .await
            .context("failed to poll device flow via server")?;

        match poll_response.status {
            DevicePollStatus::Complete => {
                let token = poll_response.login_token.ok_or_else(|| {
                    anyhow::anyhow!("server returned complete status without login_token")
                })?;
                break token;
            }
            DevicePollStatus::Pending => {
                // Continue polling
            }
            DevicePollStatus::Error => {
                let error_msg = poll_response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string());
                bail!("Device flow failed: {error_msg}");
            }
        }
    };

    config::store_auth_token(config_path, server_url, &login_token)?;

    let auth_client = HydraClient::new(client.base_url().as_str(), login_token)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClientUnauthenticated;
    use crate::config::AppConfig;
    use httpmock::prelude::*;
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn login_with_device_flow_stores_token_on_success() {
        let server = MockServer::start();

        let start_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/login/device/start");
            then.status(200).json_body(json!({
                "device_session_id": "ds-test-123",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 1
            }));
        });

        let poll_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/login/device/poll");
            then.status(200).json_body(json!({
                "status": "complete",
                "login_token": "hydra-token-abc",
                "user": {
                    "username": "octo",
                    "github_user_id": 42
                }
            }));
        });

        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body(json!({
                "actor": {"type": "user", "username": "octo"}
            }));
        });

        let client =
            HydraClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())
                .expect("client");
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let server_url = server.base_url();

        let _auth_client = login_with_github_device_flow(&client, &config_path, &server_url)
            .await
            .expect("login");

        start_mock.assert();
        poll_mock.assert();
        whoami_mock.assert();

        let stored_config = AppConfig::load(&config_path).expect("load config");
        let stored_token = stored_config
            .auth_token_for_url(&server_url)
            .expect("lookup token");
        assert_eq!(stored_token, Some("hydra-token-abc"));
    }

    #[tokio::test]
    async fn device_start_client_method_calls_correct_endpoint() {
        let server = MockServer::start();

        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/login/device/start");
            then.status(200).json_body(json!({
                "device_session_id": "ds-abc",
                "user_code": "TEST-CODE",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 600,
                "interval": 5
            }));
        });

        let client =
            HydraClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())
                .expect("client");

        let response = client.device_start().await.expect("device_start");

        mock.assert();
        assert_eq!(response.device_session_id, "ds-abc");
        assert_eq!(response.user_code, "TEST-CODE");
        assert_eq!(response.interval, 5);
    }

    #[tokio::test]
    async fn device_poll_client_method_calls_correct_endpoint() {
        let server = MockServer::start();

        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/login/device/poll")
                .json_body(json!({"device_session_id": "ds-xyz"}));
            then.status(200).json_body(json!({"status": "pending"}));
        });

        let client =
            HydraClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())
                .expect("client");

        let response = client.device_poll("ds-xyz").await.expect("device_poll");

        mock.assert();
        assert_eq!(response.status, DevicePollStatus::Pending);
        assert!(response.login_token.is_none());
    }
}
