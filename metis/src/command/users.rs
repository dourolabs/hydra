use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::whoami::ActorIdentity;
use std::io::{self, Write};

#[derive(Debug, Subcommand)]
pub enum UsersCommand {
    /// Log in with GitHub device flow.
    Login,
    /// Show information about a user.
    Info {
        /// Username to look up. Defaults to the current logged-in user.
        #[arg(value_name = "USERNAME")]
        username: Option<String>,
    },
    /// Manage per-user secrets.
    Secrets {
        #[command(subcommand)]
        command: SecretsCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum SecretsCommand {
    /// List configured secret names.
    List,
    /// Set a secret value.
    Set {
        /// Secret name (e.g., OPENAI_API_KEY).
        #[arg(value_name = "NAME")]
        name: String,
        /// Secret value. If omitted, you will be prompted to enter it.
        #[arg(long)]
        value: Option<String>,
    },
    /// Delete a secret.
    Delete {
        /// Secret name to delete.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: UsersCommand) -> Result<()> {
    match command {
        UsersCommand::Login => {
            // Login is handled during client initialization in main.rs.
            // By the time we reach here, login has already succeeded.
        }
        UsersCommand::Info { username } => {
            show_user_info(client, username).await?;
        }
        UsersCommand::Secrets { command } => {
            run_secrets(client, command).await?;
        }
    }

    Ok(())
}

async fn show_user_info(client: &dyn MetisClientInterface, username: Option<String>) -> Result<()> {
    let target_username = match username {
        Some(name) => name,
        None => {
            let whoami = client
                .whoami()
                .await
                .context("failed to fetch current user")?;
            match whoami.actor {
                ActorIdentity::User { username } => username.to_string(),
                ActorIdentity::Session { session_id, .. } => {
                    bail!("current actor is a session ({session_id}), not a user; please specify a username")
                }
                _ => {
                    bail!("current actor is not a user; please specify a username")
                }
            }
        }
    };

    let user_info = client
        .get_user_info(&target_username)
        .await
        .with_context(|| format!("failed to fetch user info for '{target_username}'"))?;

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "username: {}", user_info.username)?;
    match user_info.github_user_id {
        Some(id) => writeln!(stdout, "github_user_id: {id}")?,
        None => writeln!(stdout, "github_user_id: N/A")?,
    }

    Ok(())
}

async fn run_secrets(client: &dyn MetisClientInterface, command: SecretsCommand) -> Result<()> {
    match command {
        SecretsCommand::List => {
            let response = client
                .list_user_secrets()
                .await
                .context("failed to list secrets")?;
            if response.secrets.is_empty() {
                println!("No secrets configured.");
            } else {
                for name in &response.secrets {
                    println!("{name}");
                }
            }
        }
        SecretsCommand::Set { name, value } => {
            let value = match value {
                Some(v) => v,
                None => rpassword::prompt_password_stdout(&format!("Enter value for {name}: "))
                    .context("failed to read secret value")?,
            };
            client
                .set_user_secret(&name, &value)
                .await
                .with_context(|| format!("failed to set secret '{name}'"))?;
            println!("Secret '{name}' set successfully.");
        }
        SecretsCommand::Delete { name } => {
            client
                .delete_user_secret(&name)
                .await
                .with_context(|| format!("failed to delete secret '{name}'"))?;
            println!("Secret '{name}' deleted.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::{
        api::v1::{secrets::ListSecretsResponse, users::Username},
        users::UserSummary,
        whoami::WhoAmIResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::json;

    const TEST_METIS_TOKEN: &str = "u-test:test-metis-token";

    fn mock_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .expect("mock client creation should not fail")
    }

    #[tokio::test]
    async fn show_user_info_displays_user_details() -> Result<()> {
        let server = MockServer::start();
        let user_summary = UserSummary::new(Username::from("testuser"), Some(12345));
        let user_summary_clone = user_summary.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/testuser");
            then.status(200).json_body_obj(&user_summary_clone);
        });

        let client = mock_client(&server);
        show_user_info(&client, Some("testuser".to_string())).await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn show_user_info_uses_current_user_when_no_username_provided() -> Result<()> {
        let server = MockServer::start();
        let whoami_response = WhoAmIResponse::new(ActorIdentity::User {
            username: Username::from("currentuser"),
        });
        let user_summary = UserSummary::new(Username::from("currentuser"), Some(67890));
        let user_summary_clone = user_summary.clone();

        let whoami_mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let user_info_mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/currentuser");
            then.status(200).json_body_obj(&user_summary_clone);
        });

        let client = mock_client(&server);
        show_user_info(&client, None).await?;

        whoami_mock.assert();
        user_info_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn show_user_info_fails_when_actor_is_task() {
        let server = MockServer::start();
        let task_id = metis_common::SessionId::new();
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Session {
            session_id: task_id.clone(),
            creator: Username::from("test-creator"),
        });

        let whoami_mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let client = mock_client(&server);
        let error = show_user_info(&client, None).await.unwrap_err();

        whoami_mock.assert();
        assert!(
            error.to_string().contains("current actor is a task"),
            "error should mention actor is a task: {error}"
        );
    }

    #[tokio::test]
    async fn show_user_info_reports_user_not_found() {
        let server = MockServer::start();

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/nonexistent");
            then.status(404)
                .json_body(json!({ "error": "user not found" }));
        });

        let client = mock_client(&server);
        let error = show_user_info(&client, Some("nonexistent".to_string()))
            .await
            .unwrap_err();

        mock.assert();
        assert!(
            error.to_string().contains("failed to fetch user info"),
            "error should mention fetch failure: {error}"
        );
    }

    #[tokio::test]
    async fn secrets_list_displays_names() -> Result<()> {
        let server = MockServer::start();
        let response = ListSecretsResponse {
            secrets: vec![
                "OPENAI_API_KEY".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
            ],
        };

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/me/secrets");
            then.status(200).json_body_obj(&response);
        });

        let client = mock_client(&server);
        run_secrets(&client, SecretsCommand::List).await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn secrets_set_sends_put() -> Result<()> {
        let server = MockServer::start();

        let mock = server.mock(move |when, then| {
            when.method(PUT)
                .path("/v1/users/me/secrets/OPENAI_API_KEY")
                .json_body(json!({ "value": "sk-test123" }));
            then.status(200).json_body(json!(null));
        });

        let client = mock_client(&server);
        run_secrets(
            &client,
            SecretsCommand::Set {
                name: "OPENAI_API_KEY".to_string(),
                value: Some("sk-test123".to_string()),
            },
        )
        .await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn secrets_delete_sends_delete() -> Result<()> {
        let server = MockServer::start();

        let mock = server.mock(move |when, then| {
            when.method(DELETE)
                .path("/v1/users/me/secrets/OPENAI_API_KEY");
            then.status(200).json_body(json!(null));
        });

        let client = mock_client(&server);
        run_secrets(
            &client,
            SecretsCommand::Delete {
                name: "OPENAI_API_KEY".to_string(),
            },
        )
        .await?;

        mock.assert();
        Ok(())
    }
}
