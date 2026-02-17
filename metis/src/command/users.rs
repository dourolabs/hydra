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
                ActorIdentity::Task { task_id, .. } => {
                    bail!("current actor is a task ({task_id}), not a user; please specify a username")
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
    writeln!(stdout, "github_user_id: {}", user_info.github_user_id)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::{api::v1::users::Username, users::UserSummary, whoami::WhoAmIResponse};
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
        let user_summary = UserSummary::new(Username::from("testuser"), 12345);
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
        let user_summary = UserSummary::new(Username::from("currentuser"), 67890);
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
        let task_id = metis_common::TaskId::new();
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Task {
            task_id: task_id.clone(),
            creator: Some(Username::from("test-creator")),
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
}
