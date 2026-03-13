use crate::{
    domain::{
        actors::{Actor, ActorRef, store_github_token_secrets},
        issues::{Issue, IssueStatus, IssueType},
        jobs::{BundleSpec, Task},
        task_status::Status,
        users::{User, Username},
    },
    test_utils::{
        github_user_response, spawn_test_server_with_state, test_client_without_auth,
        test_state_handles, test_state_with_github_urls,
    },
};
use chrono::Utc;
use httpmock::prelude::*;
use metis_common::{TaskId, github::GithubTokenResponse};
use reqwest::{Client, header};
use std::collections::HashMap;

fn auth_client(token: &str) -> Client {
    let mut headers = header::HeaderMap::new();
    let auth_value = format!("Bearer {token}");
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&auth_value).expect("valid test auth header"),
    );

    Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build test client")
}

#[tokio::test]
async fn github_token_returns_for_username_actor() -> anyhow::Result<()> {
    let server = MockServer::start_async().await;
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let handles = test_state_with_github_urls(server.base_url(), server.base_url());
    let auth_token = handles
        .state
        .login_with_github_token(
            "gh-token".to_string(),
            "gh-refresh".to_string(),
            ActorRef::test(),
        )
        .await?
        .login_token;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GithubTokenResponse = response.json().await?;
    assert_eq!(body.github_token, "gh-token");

    Ok(())
}

#[tokio::test]
async fn github_token_returns_for_task_actor() -> anyhow::Result<()> {
    let server = MockServer::start_async().await;
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let handles = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User::new(username.clone(), Some(101), false);

    handles.store.add_user(user, &ActorRef::test()).await?;
    store_github_token_secrets(&handles.state, &username, "task-token", "refresh-token").await;
    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "task".to_string(),
                username.clone(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            &ActorRef::test(),
        )
        .await?;

    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        Username::from("test-creator"),
        None,
        None,
        HashMap::new(),
        None,
        None,
        None,
        Status::Created,
        None,
        None,
    );
    let (task_id, _) = handles
        .store
        .add_task(task, Utc::now(), &ActorRef::test())
        .await?;
    let (actor, auth_token) = Actor::new_for_session(task_id, Username::from("creator"));
    handles.store.add_actor(actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GithubTokenResponse = response.json().await?;
    assert_eq!(body.github_token, "task-token");

    Ok(())
}

#[tokio::test]
async fn github_token_requires_auth() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn github_token_returns_not_found_for_missing_user() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let (actor, auth_token) = Actor::new_for_user(Username::from("octo"));
    handles.store.add_actor(actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn github_token_refreshes_expired_token() -> anyhow::Result<()> {
    let server = MockServer::start_async().await;
    let _user_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/user")
            .header("authorization", "Bearer expired-token");
        then.status(401);
    });
    let refresh_mock = server.mock(|when, then| {
        when.method(POST)
            .path("/login/oauth/access_token")
            .header("accept", "application/json")
            .body_contains("grant_type=refresh_token")
            .body_contains("refresh_token=refresh-token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "access_token": "new-token",
                "refresh_token": "new-refresh"
            }));
    });

    let handles = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User {
        username: username.clone(),
        github_user_id: Some(101),
        deleted: false,
    };

    handles.store.add_user(user, &ActorRef::test()).await?;
    store_github_token_secrets(&handles.state, &username, "expired-token", "refresh-token").await;
    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "task".to_string(),
                username.clone(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            &ActorRef::test(),
        )
        .await?;

    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        Username::from("test-creator"),
        None,
        None,
        HashMap::new(),
        None,
        None,
        None,
        Status::Created,
        None,
        None,
    );
    let (task_id, _) = handles
        .store
        .add_task(task, Utc::now(), &ActorRef::test())
        .await?;
    let (actor, auth_token) = Actor::new_for_session(task_id, Username::from("creator"));
    handles.store.add_actor(actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GithubTokenResponse = response.json().await?;
    assert_eq!(body.github_token, "new-token");

    refresh_mock.assert();

    Ok(())
}

#[tokio::test]
async fn github_token_refresh_failure_returns_unauthorized() -> anyhow::Result<()> {
    let server = MockServer::start_async().await;
    let _user_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/user")
            .header("authorization", "Bearer expired-token");
        then.status(401);
    });
    let _refresh_mock = server.mock(|when, then| {
        when.method(POST)
            .path("/login/oauth/access_token")
            .header("accept", "application/json")
            .body_contains("grant_type=refresh_token")
            .body_contains("refresh_token=bad-refresh");
        then.status(400)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "bad refresh token"
            }));
    });

    let handles = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User {
        username: username.clone(),
        github_user_id: Some(101),
        deleted: false,
    };

    handles.store.add_user(user, &ActorRef::test()).await?;
    store_github_token_secrets(&handles.state, &username, "expired-token", "bad-refresh").await;
    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "task".to_string(),
                username.clone(),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            &ActorRef::test(),
        )
        .await?;

    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        Username::from("test-creator"),
        None,
        None,
        HashMap::new(),
        None,
        None,
        None,
        Status::Created,
        None,
        None,
    );
    let (task_id, _) = handles
        .store
        .add_task(task, Utc::now(), &ActorRef::test())
        .await?;
    let (actor, auth_token) = Actor::new_for_session(task_id, Username::from("creator"));
    handles.store.add_actor(actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn github_token_returns_not_found_for_missing_task() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let task_id = TaskId::new();
    let (actor, auth_token) = Actor::new_for_session(task_id, Username::from("creator"));
    handles.store.add_actor(actor, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    Ok(())
}
