use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::users::{CreateUserRequest, UpdateGithubTokenRequest, UserSummary, Username};
use std::io::{self, Write};

#[derive(Debug, Subcommand)]
pub enum UsersCommand {
    /// List configured users.
    List {
        /// Pretty-print users instead of emitting JSONL.
        #[arg(long)]
        pretty: bool,
    },
    /// Create a new user.
    Add(UserCredentialsArgs),
    /// Delete an existing user.
    Delete {
        /// Username to delete.
        #[arg(value_name = "USERNAME")]
        username: String,
    },
    /// Update the GitHub token for a user.
    SetGithubToken(UserCredentialsArgs),
}

#[derive(Debug, Clone, Args)]
pub struct UserCredentialsArgs {
    /// Username for the account.
    #[arg(value_name = "USERNAME")]
    pub username: String,

    /// GitHub token for the account.
    #[arg(value_name = "TOKEN")]
    pub github_token: String,
}

pub async fn run(client: &dyn MetisClientInterface, command: UsersCommand) -> Result<()> {
    match command {
        UsersCommand::List { pretty } => {
            let users = fetch_users(client).await?;
            let mut stdout = io::stdout().lock();
            if pretty {
                print_users_pretty(&users, &mut stdout)?;
            } else {
                print_users_jsonl(&users, &mut stdout)?;
            }
        }
        UsersCommand::Add(args) => {
            let user = create_user(client, args).await?;
            let mut stdout = io::stdout().lock();
            print_user_action("Created user", &user, &mut stdout)?;
        }
        UsersCommand::Delete { username } => {
            let deleted = delete_user(client, &username).await?;
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "Deleted user: {deleted}")?;
            stdout.flush()?;
        }
        UsersCommand::SetGithubToken(args) => {
            let user = set_github_token(client, args).await?;
            let mut stdout = io::stdout().lock();
            print_user_action("Updated GitHub token", &user, &mut stdout)?;
        }
    }

    Ok(())
}

async fn fetch_users(client: &dyn MetisClientInterface) -> Result<Vec<UserSummary>> {
    let response = client.list_users().await.context("failed to list users")?;
    Ok(response.users)
}

async fn create_user(
    client: &dyn MetisClientInterface,
    args: UserCredentialsArgs,
) -> Result<UserSummary> {
    let request = CreateUserRequest {
        username: normalize_non_empty(&args.username, "username")?.into(),
        github_user_id: None,
        github_username: None,
        github_token: normalize_non_empty(&args.github_token, "github token")?,
    };
    let response = client
        .create_user(&request)
        .await
        .context("failed to create user")?;
    Ok(response.user)
}

async fn delete_user(client: &dyn MetisClientInterface, username: &str) -> Result<Username> {
    let username: Username = normalize_non_empty(username, "username")?.into();
    let response = client
        .delete_user(&username)
        .await
        .context("failed to delete user")?;
    Ok(response.username)
}

async fn set_github_token(
    client: &dyn MetisClientInterface,
    args: UserCredentialsArgs,
) -> Result<UserSummary> {
    let username: Username = normalize_non_empty(&args.username, "username")?.into();
    let request = UpdateGithubTokenRequest {
        github_token: normalize_non_empty(&args.github_token, "github token")?,
        github_user_id: None,
        github_username: None,
    };
    let response = client
        .set_user_github_token(&username, &request)
        .await
        .context("failed to update GitHub token")?;
    Ok(response.user)
}

fn normalize_non_empty(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field} must not be empty");
    }

    Ok(trimmed.to_string())
}

fn print_users_jsonl(users: &[UserSummary], writer: &mut impl Write) -> Result<()> {
    for user in users {
        serde_json::to_writer(&mut *writer, user)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn print_users_pretty(users: &[UserSummary], writer: &mut impl Write) -> Result<()> {
    if users.is_empty() {
        writeln!(writer, "No users configured.")?;
        writer.flush()?;
        return Ok(());
    }

    writeln!(writer, "Configured users:")?;
    for user in users {
        writeln!(writer, "  - {}", user.username)?;
    }
    writer.flush()?;
    Ok(())
}

fn print_user_action(action: &str, user: &UserSummary, writer: &mut impl Write) -> Result<()> {
    writeln!(writer, "{action}: {}", user.username)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::users::{DeleteUserResponse, ListUsersResponse, UpsertUserResponse};
    use serde_json::json;

    #[tokio::test]
    async fn list_users_prints_jsonl_without_tokens() {
        let server = MockServer::start();
        let payload = ListUsersResponse {
            users: vec![
                UserSummary {
                    username: Username::from("alice"),
                    github_user_id: None,
                    github_username: None,
                },
                UserSummary {
                    username: Username::from("bob"),
                    github_user_id: None,
                    github_username: None,
                },
            ],
        };

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/users");
            then.status(200).json_body_obj(&payload);
        });

        let client = MetisClient::new(server.base_url()).unwrap();

        let users = fetch_users(&client).await.unwrap();
        mock.assert();

        let mut output = Vec::new();
        print_users_jsonl(&users, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"username\":\"alice\""));
        assert!(output.contains("\"username\":\"bob\""));
        assert!(!output.contains("github_token"));
    }

    #[tokio::test]
    async fn add_user_sends_request_and_prints_result() {
        let server = MockServer::start();
        let client = MetisClient::new(server.base_url()).unwrap();
        let args = UserCredentialsArgs {
            username: "alice".to_string(),
            github_token: "token-123".to_string(),
        };
        let response = UpsertUserResponse {
            user: UserSummary {
                username: Username::from("alice"),
                github_user_id: None,
                github_username: None,
            },
        };
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/users").json_body(json!({
                "username": "alice",
                "github_token": "token-123"
            }));
            then.status(200).json_body_obj(&response);
        });

        let user = create_user(&client, args.clone()).await.unwrap();
        mock.assert();

        let mut output = Vec::new();
        print_user_action("Created user", &user, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Created user: alice"));
    }

    #[tokio::test]
    async fn delete_user_trims_username() {
        let server = MockServer::start();
        let client = MetisClient::new(server.base_url()).unwrap();
        let mock = server.mock(|when, then| {
            when.method(DELETE).path("/v1/users/alice");
            then.status(200).json_body_obj(&DeleteUserResponse {
                username: Username::from("alice"),
            });
        });

        let deleted = delete_user(&client, "  alice ").await.unwrap();
        mock.assert();
        assert_eq!(deleted.as_str(), "alice");
    }

    #[tokio::test]
    async fn set_github_token_rejects_empty_token() {
        let server = MockServer::start();
        let client = MetisClient::new(server.base_url()).unwrap();
        let args = UserCredentialsArgs {
            username: "alice".to_string(),
            github_token: "   ".to_string(),
        };

        let err = set_github_token(&client, args).await.unwrap_err();
        assert!(err.to_string().contains("github token must not be empty"));
    }
}
