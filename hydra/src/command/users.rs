use super::utils::resolve_username;
use crate::client::HydraClientInterface;
use crate::command::output::{render, CommandContext, UserRecords};
use crate::output_writer::write_stdout;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::api::v1::users::SearchUsersQuery;
use hydra_common::whoami::ActorIdentity;

#[derive(Debug, Subcommand)]
pub enum UsersCommand {
    /// Log in with GitHub device flow.
    Login,
    /// List users.
    List(ListUsersArgs),
    /// Get details for a user.
    Get {
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

#[derive(Debug, Clone, Args)]
pub struct ListUsersArgs {
    /// Filter users by a free-text query (case-insensitive substring match).
    #[arg(long = "q", value_name = "QUERY")]
    pub q: Option<String>,

    /// Include soft-deleted users in the results.
    #[arg(long = "include-deleted")]
    pub include_deleted: bool,
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

pub async fn run(
    client: &dyn HydraClientInterface,
    command: UsersCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut buffer = Vec::new();
    match command {
        UsersCommand::Login => {
            // Login is handled during client initialization in main.rs.
            // By the time we reach here, login has already succeeded.
        }
        UsersCommand::List(args) => {
            let users = list_users(client, args).await?;
            render(UserRecords(&users), context.output_format, &mut buffer)?;
        }
        UsersCommand::Get { username } => {
            let user = get_user(client, username).await?;
            render(UserRecords(&[user]), context.output_format, &mut buffer)?;
        }
        UsersCommand::Secrets { command } => {
            run_secrets(client, command).await?;
        }
    }
    if !buffer.is_empty() {
        write_stdout(&buffer)?;
    }

    Ok(())
}

async fn list_users(
    client: &dyn HydraClientInterface,
    args: ListUsersArgs,
) -> Result<Vec<hydra_common::users::UserSummary>> {
    let query = SearchUsersQuery::new(args.q, args.include_deleted.then_some(true));
    let response = client
        .list_users(&query)
        .await
        .context("failed to list users")?;
    Ok(response.users)
}

async fn get_user(
    client: &dyn HydraClientInterface,
    username: Option<String>,
) -> Result<hydra_common::users::UserSummary> {
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

    client
        .get_user(&target_username)
        .await
        .with_context(|| format!("failed to fetch user info for '{target_username}'"))
}

async fn run_secrets(client: &dyn HydraClientInterface, command: SecretsCommand) -> Result<()> {
    let username = resolve_username(client).await?;
    match command {
        SecretsCommand::List => {
            let response = client
                .list_user_secrets(username.as_ref())
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
                .set_user_secret(username.as_ref(), &name, &value)
                .await
                .with_context(|| format!("failed to set secret '{name}'"))?;
            println!("Secret '{name}' set successfully.");
        }
        SecretsCommand::Delete { name } => {
            client
                .delete_user_secret(username.as_ref(), &name)
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
    use crate::client::HydraClient;
    use crate::command::output::{render, ResolvedOutputFormat, UserRecords};
    use httpmock::prelude::*;
    use hydra_common::{
        api::v1::{
            secrets::ListSecretsResponse,
            users::{ListUsersResponse, Username},
        },
        users::UserSummary,
        whoami::WhoAmIResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::json;

    const TEST_HYDRA_TOKEN: &str = "u-test:test-hydra-token";

    fn mock_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .expect("mock client creation should not fail")
    }

    #[tokio::test]
    async fn get_user_displays_user_details() -> Result<()> {
        let server = MockServer::start();
        let user_summary = UserSummary::new(Username::from("testuser"), Some(12345));
        let user_summary_clone = user_summary.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/testuser");
            then.status(200).json_body_obj(&user_summary_clone);
        });

        let client = mock_client(&server);
        let user = get_user(&client, Some("testuser".to_string())).await?;

        mock.assert();
        assert_eq!(user, user_summary);
        Ok(())
    }

    #[tokio::test]
    async fn get_user_uses_current_user_when_no_username_provided() -> Result<()> {
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
        let user = get_user(&client, None).await?;

        whoami_mock.assert();
        user_info_mock.assert();
        assert_eq!(user.username.as_str(), "currentuser");
        Ok(())
    }

    #[tokio::test]
    async fn get_user_fails_when_actor_is_session() {
        let server = MockServer::start();
        let task_id = hydra_common::SessionId::new();
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Session {
            session_id: task_id.clone(),
            creator: Username::from("test-creator"),
        });

        let whoami_mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let client = mock_client(&server);
        let error = get_user(&client, None).await.unwrap_err();

        whoami_mock.assert();
        assert!(
            error.to_string().contains("current actor is a session"),
            "error should mention actor is a session: {error}"
        );
    }

    #[tokio::test]
    async fn get_user_reports_user_not_found() {
        let server = MockServer::start();

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/nonexistent");
            then.status(404)
                .json_body(json!({ "error": "user not found" }));
        });

        let client = mock_client(&server);
        let error = get_user(&client, Some("nonexistent".to_string()))
            .await
            .unwrap_err();

        mock.assert();
        assert!(
            error.to_string().contains("failed to fetch user info"),
            "error should mention fetch failure: {error}"
        );
    }

    #[tokio::test]
    async fn get_user_pretty_output_shows_user_details() -> Result<()> {
        let user = UserSummary::new(Username::from("alice"), Some(42));
        let mut output = Vec::new();
        render(
            UserRecords(&[user]),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("alice"));
        assert!(output.contains("github_user_id: 42"));
        Ok(())
    }

    #[tokio::test]
    async fn get_user_jsonl_output_emits_one_record() -> Result<()> {
        let user = UserSummary::new(Username::from("alice"), Some(42));
        let mut output = Vec::new();
        render(
            UserRecords(&[user]),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("\"username\":\"alice\""));
        assert_eq!(output.lines().count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn list_users_fetches_users_and_prints_jsonl() -> Result<()> {
        let server = MockServer::start();
        let list_response = ListUsersResponse::new(vec![
            UserSummary::new(Username::from("alice"), Some(1)),
            UserSummary::new(Username::from("bob"), None),
        ]);

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users");
            then.status(200).json_body_obj(&list_response);
        });

        let client = mock_client(&server);
        let users = list_users(
            &client,
            ListUsersArgs {
                q: None,
                include_deleted: false,
            },
        )
        .await?;
        mock.assert();

        let mut output = Vec::new();
        render(
            UserRecords(&users),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("\"username\":\"alice\""));
        assert!(output.contains("\"username\":\"bob\""));
        assert_eq!(output.lines().count(), 2);

        Ok(())
    }

    #[tokio::test]
    async fn list_users_prints_pretty_format() -> Result<()> {
        let users = vec![
            UserSummary::new(Username::from("alice"), Some(1)),
            UserSummary::new(Username::from("bob"), None),
        ];
        let mut output = Vec::new();

        render(
            UserRecords(&users),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("alice"));
        assert!(output.contains("github_user_id: 1"));
        assert!(output.contains("bob"));
        assert!(output.contains("github_user_id: N/A"));

        Ok(())
    }

    #[tokio::test]
    async fn list_users_empty_pretty_shows_message() -> Result<()> {
        let users: Vec<UserSummary> = Vec::new();
        let mut output = Vec::new();
        render(
            UserRecords(&users),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("No users found."));
        Ok(())
    }

    #[tokio::test]
    async fn list_users_passes_q_and_include_deleted() -> Result<()> {
        let server = MockServer::start();
        let list_response =
            ListUsersResponse::new(vec![UserSummary::new(Username::from("alice"), Some(1))]);

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/users")
                .query_param("q", "alice")
                .query_param("include_deleted", "true");
            then.status(200).json_body_obj(&list_response);
        });

        let client = mock_client(&server);
        let users = list_users(
            &client,
            ListUsersArgs {
                q: Some("alice".to_string()),
                include_deleted: true,
            },
        )
        .await?;
        mock.assert();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username.as_str(), "alice");
        Ok(())
    }

    fn mock_whoami(server: &MockServer) {
        let whoami_response = WhoAmIResponse::new(ActorIdentity::User {
            username: Username::from("testuser"),
        });
        server.mock(move |when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });
    }

    #[tokio::test]
    async fn secrets_list_displays_names() -> Result<()> {
        let server = MockServer::start();
        mock_whoami(&server);
        let response = ListSecretsResponse {
            secrets: vec![
                "OPENAI_API_KEY".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
            ],
        };

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/testuser/secrets");
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
        mock_whoami(&server);

        let mock = server.mock(move |when, then| {
            when.method(PUT)
                .path("/v1/users/testuser/secrets/OPENAI_API_KEY")
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
        mock_whoami(&server);

        let mock = server.mock(move |when, then| {
            when.method(DELETE)
                .path("/v1/users/testuser/secrets/OPENAI_API_KEY");
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
