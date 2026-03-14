pub mod changelog;

use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use metis_common::{users::Username, whoami::ActorIdentity};

/// Resolve the username of the currently authenticated actor via whoami.
pub async fn resolve_username(client: &dyn MetisClientInterface) -> Result<Username> {
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
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::{users::Username, whoami::WhoAmIResponse, SessionId};
    use reqwest::Client as HttpClient;
    use std::str::FromStr;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn metis_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .unwrap()
    }

    #[tokio::test]
    async fn resolve_username_uses_whoami_user() {
        let server = MockServer::start();
        let client = metis_client(&server);
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
        let client = metis_client(&server);
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
