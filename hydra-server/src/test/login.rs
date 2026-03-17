use crate::{
    domain::users::Username,
    test::{
        github_user_response, spawn_test_server_with_state, test_client,
        test_state_with_github_api_base_url, test_state_with_github_urls_and_allowed_orgs,
    },
};
use httpmock::prelude::*;
use hydra_common::api::v1::login::LoginRequest;
use reqwest::StatusCode;
use serde_json::Value;

#[tokio::test]
async fn login_creates_actor_and_returns_token() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
    let check_store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
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

    let user = check_store.get_user(&Username::from("octo"), false).await?;
    assert_eq!(user.item.username.as_str(), "octo");
    assert_eq!(user.item.github_user_id, Some(42));

    let actors = check_store.list_actors().await?;
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

    let handles = test_state_with_github_api_base_url(github_server.base_url());
    let check_store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = test_client();

    let payload = LoginRequest::new("gh-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    // Verify the user was persisted.
    check_store.get_user(&Username::from("octo"), false).await?;

    Ok(())
}

#[tokio::test]
async fn login_rejects_empty_token() -> anyhow::Result<()> {
    let handles = test_state_with_github_api_base_url("https://example.invalid".to_string());
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
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

    let handles = test_state_with_github_api_base_url(github_server.base_url());
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
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

#[tokio::test]
async fn login_allows_user_in_allowed_orgs() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });
    let _orgs_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user/orgs");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!([{ "login": "hydra" }]));
    });

    let handles = test_state_with_github_urls_and_allowed_orgs(
        github_server.base_url(),
        github_server.base_url(),
        vec!["hydra".to_string()],
    );
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let payload = LoginRequest::new("gh-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn login_rejects_user_not_in_allowed_orgs() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });
    let _orgs_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user/orgs");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!([{ "login": "another-org" }]));
    });

    let handles = test_state_with_github_urls_and_allowed_orgs(
        github_server.base_url(),
        github_server.base_url(),
        vec!["hydra".to_string()],
    );
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let payload = LoginRequest::new("gh-token".to_string(), "gh-refresh".to_string());
    let response = client
        .post(format!("{}/v1/login", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}
