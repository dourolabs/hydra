use crate::client::MetisClientInterface;
use crate::command::users;
use anyhow::Result;
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum SecretsCommand {
    /// List configured secret names for the current user.
    List,
}

pub async fn run(client: &dyn MetisClientInterface, command: SecretsCommand) -> Result<()> {
    match command {
        SecretsCommand::List => {
            users::run_secrets(client, users::SecretsCommand::List).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::api::v1::secrets::ListSecretsResponse;
    use reqwest::Client as HttpClient;

    const TEST_METIS_TOKEN: &str = "u-test:test-metis-token";

    fn mock_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .expect("mock client creation should not fail")
    }

    #[tokio::test]
    async fn secrets_list_shows_names() -> Result<()> {
        let server = MockServer::start();
        let response = ListSecretsResponse {
            secrets: vec!["OPENAI_API_KEY".to_string(), "MY_CUSTOM_SECRET".to_string()],
        };

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/me/secrets");
            then.status(200).json_body_obj(&response);
        });

        let client = mock_client(&server);
        run(&client, SecretsCommand::List).await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn secrets_list_empty() -> Result<()> {
        let server = MockServer::start();
        let response = ListSecretsResponse { secrets: vec![] };

        let mock = server.mock(move |when, then| {
            when.method(GET).path("/v1/users/me/secrets");
            then.status(200).json_body_obj(&response);
        });

        let client = mock_client(&server);
        run(&client, SecretsCommand::List).await?;

        mock.assert();
        Ok(())
    }
}
