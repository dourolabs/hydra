use crate::test::{spawn_test_server_with_state, test_client, test_state_with_github_client};
use httpmock::prelude::*;
use metis_common::api::v1::login::LoginRequest;
use octocrab::Octocrab;
use reqwest::StatusCode;
use serde_json::{Value, json};

fn github_user_response(login: &str, id: u64) -> serde_json::Value {
    json!({
        "login": login,
        "id": id,
        "node_id": "NODEID",
        "avatar_url": "https://example.com/avatar",
        "gravatar_id": "gravatar",
        "url": "https://example.com/user",
        "html_url": "https://example.com/user",
        "followers_url": "https://example.com/followers",
        "following_url": "https://example.com/following",
        "gists_url": "https://example.com/gists",
        "starred_url": "https://example.com/starred",
        "subscriptions_url": "https://example.com/subscriptions",
        "organizations_url": "https://example.com/orgs",
        "repos_url": "https://example.com/repos",
        "events_url": "https://example.com/events",
        "received_events_url": "https://example.com/received_events",
        "type": "User",
        "site_admin": false,
        "name": null,
        "patch_url": null,
        "email": null
    })
}

fn build_github_client(base_url: String) -> Octocrab {
    Octocrab::builder()
        .base_uri(base_url)
        .unwrap()
        .personal_token("gh-token".to_string())
        .build()
        .unwrap()
}

#[tokio::test]
async fn login_creates_actor_and_returns_token() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let state = test_state_with_github_client(build_github_client(github_server.base_url()));
    let store = state.store.clone();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = LoginRequest::new("gh-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = response.json().await?;
    assert!(body.get("login_token").and_then(Value::as_str).is_some());
    assert_eq!(
        body.get("user")
            .and_then(|user| user.get("username"))
            .and_then(Value::as_str),
        Some("octo")
    );
    assert_eq!(
        body.get("user")
            .and_then(|user| user.get("github_user_id"))
            .and_then(Value::as_u64),
        Some(42)
    );

    let store_read = store.read().await;
    let users = store_read.list_users().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].username.as_str(), "octo");
    assert_eq!(users[0].github_user_id, 42);
    assert_eq!(users[0].github_refresh_token, "gh-refresh");

    let actors = store_read.list_actors().await?;
    assert!(
        actors.iter().any(|(name, _)| name == "u-octo"),
        "expected login actor to be created"
    );

    Ok(())
}

#[tokio::test]
async fn login_persists_refresh_token() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let state = test_state_with_github_client(build_github_client(github_server.base_url()));
    let store = state.store.clone();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = LoginRequest::new("gh-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let store_read = store.read().await;
    let users = store_read.list_users().await?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].github_refresh_token, "gh-refresh");

    Ok(())
}

#[tokio::test]
async fn login_rejects_empty_token() -> anyhow::Result<()> {
    let state =
        test_state_with_github_client(build_github_client("https://example.invalid".to_string()));
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = LoginRequest::new("  ".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn login_returns_bad_request_for_invalid_token() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(401);
    });

    let state = test_state_with_github_client(build_github_client(github_server.base_url()));
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = LoginRequest::new("bad-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
