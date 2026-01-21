use crate::client::MetisClientInterface;
use crate::github_oauth::{DeviceCodeResponse, GitHubOAuthDeviceFlow};
use anyhow::{bail, Context, Result};
use clap::Args;
use metis_common::constants::ENV_METIS_GITHUB_CLIENT_ID;
use metis_common::users::{CreateUserRequest, UpdateGithubTokenRequest, UserSummary, Username};
use reqwest::Client as HttpClient;
use std::io::{self, Write};

const GITHUB_OAUTH_BASE_URL: &str = "https://github.com";
const GITHUB_API_BASE_URL: &str = "https://api.github.com";
const METIS_GITHUB_USER_AGENT: &str = "metis-cli";

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// GitHub OAuth client ID.
    #[arg(long, env = ENV_METIS_GITHUB_CLIENT_ID, value_name = "CLIENT_ID")]
    pub github_client_id: String,
}

pub async fn run(client: &dyn MetisClientInterface, args: LoginArgs) -> Result<()> {
    let oauth = GitHubOAuthDeviceFlow::new(
        HttpClient::new(),
        GITHUB_OAUTH_BASE_URL,
        GITHUB_API_BASE_URL,
    )
    .context("failed to configure GitHub OAuth device flow")?;
    let device = oauth
        .request_device_code(&args.github_client_id)
        .await
        .context("failed to request GitHub device code")?;

    let mut stdout = io::stdout().lock();
    print_device_prompt(&device, &mut stdout)?;

    let (username, token) = oauth
        .complete_device_flow(&args.github_client_id, &device, METIS_GITHUB_USER_AGENT)
        .await
        .context("failed to complete GitHub device flow")?;
    let user = upsert_user(client, &username, &token).await?;
    print_login_success(&user, &mut stdout)?;

    Ok(())
}

async fn upsert_user(
    client: &dyn MetisClientInterface,
    github_username: &str,
    github_token: &str,
) -> Result<UserSummary> {
    let username = normalize_username(github_username)?;
    let github_token = normalize_non_empty(github_token, "GitHub token")?;

    let users = client.list_users().await.context("failed to list users")?;
    if users
        .users
        .iter()
        .any(|user| user.username.as_str() == username.as_str())
    {
        let request = UpdateGithubTokenRequest { github_token };
        let response = client
            .set_user_github_token(&username, &request)
            .await
            .context("failed to update GitHub token")?;
        Ok(response.user)
    } else {
        let request = CreateUserRequest {
            username,
            github_token,
        };
        let response = client
            .create_user(&request)
            .await
            .context("failed to create user")?;
        Ok(response.user)
    }
}

fn normalize_username(value: &str) -> Result<Username> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("GitHub username must not be empty");
    }

    Ok(Username::from(trimmed))
}

fn normalize_non_empty(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field} must not be empty");
    }

    Ok(trimmed.to_string())
}

fn print_device_prompt(device: &DeviceCodeResponse, writer: &mut impl Write) -> Result<()> {
    writeln!(
        writer,
        "Open {} and enter code {} to authorize GitHub.",
        device.verification_uri, device.user_code
    )?;
    writer.flush()?;
    Ok(())
}

fn print_login_success(user: &UserSummary, writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "Logged in as {}", user.username)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use metis_common::users::{ListUsersResponse, UpsertUserResponse};

    #[tokio::test]
    async fn upsert_user_creates_when_missing() {
        let client = MockMetisClient::default();
        client.push_list_users_response(ListUsersResponse { users: Vec::new() });
        client.push_create_user_response(UpsertUserResponse {
            user: UserSummary {
                username: Username::from("octocat"),
            },
        });

        let user = upsert_user(&client, "octocat", "token-123")
            .await
            .expect("upsert user");

        assert_eq!(client.recorded_list_users_calls(), 1);
        let create_requests = client.recorded_create_user_requests();
        assert_eq!(create_requests.len(), 1);
        assert_eq!(create_requests[0].username.as_str(), "octocat");
        assert_eq!(create_requests[0].github_token, "token-123");
        assert!(client.recorded_set_user_github_token_requests().is_empty());
        assert_eq!(user.username.as_str(), "octocat");
    }

    #[tokio::test]
    async fn upsert_user_updates_when_present() {
        let client = MockMetisClient::default();
        client.push_list_users_response(ListUsersResponse {
            users: vec![UserSummary {
                username: Username::from("octocat"),
            }],
        });
        client.push_set_user_github_token_response(UpsertUserResponse {
            user: UserSummary {
                username: Username::from("octocat"),
            },
        });

        let user = upsert_user(&client, "octocat", "token-456")
            .await
            .expect("upsert user");

        assert_eq!(client.recorded_list_users_calls(), 1);
        assert!(client.recorded_create_user_requests().is_empty());
        let update_requests = client.recorded_set_user_github_token_requests();
        assert_eq!(update_requests.len(), 1);
        assert_eq!(update_requests[0].0.as_str(), "octocat");
        assert_eq!(update_requests[0].1.github_token, "token-456");
        assert_eq!(user.username.as_str(), "octocat");
    }

    #[test]
    fn print_login_success_includes_username() {
        let mut output = Vec::new();
        let user = UserSummary {
            username: Username::from("octocat"),
        };

        print_login_success(&user, &mut output).expect("print login success");
        let output = String::from_utf8(output).expect("utf8 output");

        assert!(output.contains("Logged in as octocat"));
    }
}
