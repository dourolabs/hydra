use crate::test::{
    github_user_response, spawn_test_server, spawn_test_server_with_state, test_auth_token,
    test_client, test_state_with_github_api_base_url,
};
use httpmock::prelude::*;
use metis_common::api::v1::whoami::{ActorIdentity, WhoAmIResponse};
use reqwest::{Client, StatusCode, header};

fn client_with_token(token: &str) -> Client {
    let mut headers = header::HeaderMap::new();
    let auth_value = format!("Bearer {token}");
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&auth_value).expect("valid auth header"),
    );

    Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build client")
}

#[tokio::test]
async fn whoami_returns_user_identity() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let state = test_state_with_github_api_base_url(github_server.base_url());
    let token = state
        .login_with_github_token("gh-token".to_string(), "gh-refresh".to_string())
        .await?
        .login_token;

    let server = spawn_test_server_with_state(state).await?;
    let client = client_with_token(&token);
    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::User { username } => {
            assert_eq!(username.as_str(), "octo");
        }
        other => {
            panic!("expected user identity, got {other:?}");
        }
    }

    Ok(())
}

#[tokio::test]
async fn whoami_returns_task_identity() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let token = test_auth_token();
    let actor_name = token
        .split_once(':')
        .map(|(name, _)| name)
        .expect("expected token to include actor name");
    let expected_task_id = actor_name
        .strip_prefix("w-")
        .expect("expected worker actor token");

    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: WhoAmIResponse = response.json().await?;
    match body.actor {
        ActorIdentity::Task { task_id } => {
            assert_eq!(task_id.as_ref(), expected_task_id);
        }
        other => {
            panic!("expected task identity, got {other:?}");
        }
    }

    Ok(())
}
