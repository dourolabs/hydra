pub mod changelog;

use crate::client::HydraClientInterface;
use anyhow::{bail, Context, Result};
use hydra_common::{users::Username, whoami::ActorIdentity};

/// Resolve the username of the currently authenticated actor via whoami.
pub async fn resolve_username(client: &dyn HydraClientInterface) -> Result<Username> {
    let response = client
        .whoami()
        .await
        .context("failed to resolve authenticated actor")?;
    match response.actor {
        ActorIdentity::User { username } => Ok(username),
        ActorIdentity::Session { creator, .. } | ActorIdentity::Issue { creator, .. } => {
            Ok(creator)
        }
        other => bail!("unexpected actor identity: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use httpmock::prelude::*;
    use hydra_common::{users::Username, whoami::WhoAmIResponse, SessionId};
    use reqwest::Client as HttpClient;
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn hydra_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .unwrap()
    }

    #[tokio::test]
    async fn resolve_username_uses_whoami_user() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let whoami_response = WhoAmIResponse::new(ActorIdentity::User {
            username: Username::from("creator-a"),
        });
        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let username = resolve_username(&client).await.unwrap();

        assert_eq!(username, Username::from("creator-a"));
        whoami_mock.assert();
        assert_eq!(whoami_mock.hits(), 1);
    }

    #[tokio::test]
    async fn resolve_username_uses_whoami_creator_for_session() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let whoami_response = WhoAmIResponse::new(ActorIdentity::Session {
            session_id: SessionId::from_str("s-abcd").unwrap(),
            creator: Username::from("whoami-creator"),
        });
        let whoami_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami_response);
        });

        let username = resolve_username(&client).await.unwrap();

        assert_eq!(username, Username::from("whoami-creator"));
        whoami_mock.assert();
        assert_eq!(whoami_mock.hits(), 1);
    }
}
