use crate::{
    app::{AppState, ServiceState},
    domain::{
        actors::Actor,
        issues::{Issue, IssueStatus, IssueType},
        jobs::{BundleSpec, Task},
        users::{User, Username},
    },
    store::MemoryStore,
    test_utils::{
        MockJobEngine, spawn_test_server_with_state, test_app_config, test_client_without_auth,
        test_state,
    },
};
use chrono::Utc;
use httpmock::prelude::*;
use metis_common::{TaskId, github::GithubTokenResponse};
use octocrab::Octocrab;
use reqwest::{Client, header};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

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

fn github_user_response(login: &str, id: u64) -> serde_json::Value {
    serde_json::json!({
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

fn test_state_with_github_urls(api_base_url: String, oauth_base_url: String) -> AppState {
    let mut config = test_app_config();
    config.github_app.api_base_url = api_base_url;
    config.github_app.oauth_base_url = oauth_base_url;

    AppState {
        config: Arc::new(config),
        github_app: None,
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine: Arc::new(MockJobEngine::new()),
        agents: Arc::new(RwLock::new(Vec::new())),
    }
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

    let github_client = build_github_client(server.base_url());
    let (user, actor, auth_token) = Actor::new_for_github_token_with_client(
        "gh-token".to_string(),
        "gh-refresh".to_string(),
        &github_client,
    )
    .await?;

    let state = test_state_with_github_urls(server.base_url(), server.base_url());
    state.add_user(user).await?;
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state).await?;
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

    let state = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User::new(
        username.clone(),
        101,
        "task-token".to_string(),
        "refresh-token".to_string(),
    );

    state.add_user(user).await?;
    let issue_id = state
        .add_issue(Issue::new(
            IssueType::Task,
            "task".to_string(),
            username.clone(),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .await?;

    let task_id = TaskId::new();
    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        None,
        HashMap::new(),
        None,
        None,
    );
    let (actor, auth_token) = Actor::new_for_task(task_id.clone());
    state.add_task_with_id(task_id, task, Utc::now()).await?;
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state).await?;
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
    let state = test_state();
    let server = spawn_test_server_with_state(state).await?;
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
    let server = MockServer::start_async().await;
    let _mock = server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let github_client = build_github_client(server.base_url());
    let (_user, actor, auth_token) = Actor::new_for_github_token_with_client(
        "gh-token".to_string(),
        "gh-refresh".to_string(),
        &github_client,
    )
    .await?;

    let state = test_state();
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state).await?;
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

    let state = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User {
        username: username.clone(),
        github_user_id: 101,
        github_token: "expired-token".to_string(),
        github_refresh_token: "refresh-token".to_string(),
    };

    state.add_user(user).await?;
    let issue_id = state
        .add_issue(Issue::new(
            IssueType::Task,
            "task".to_string(),
            username.clone(),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .await?;

    let task_id = TaskId::new();
    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        None,
        HashMap::new(),
        None,
        None,
    );
    let (actor, auth_token) = Actor::new_for_task(task_id.clone());
    state.add_task_with_id(task_id, task, Utc::now()).await?;
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state.clone()).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GithubTokenResponse = response.json().await?;
    assert_eq!(body.github_token, "new-token");

    let updated = state.get_user(&username).await?;
    assert_eq!(updated.github_token, "new-token");
    assert_eq!(updated.github_refresh_token, "new-refresh");
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

    let state = test_state_with_github_urls(server.base_url(), server.base_url());
    let username = Username::from("creator");
    let user = User {
        username: username.clone(),
        github_user_id: 101,
        github_token: "expired-token".to_string(),
        github_refresh_token: "bad-refresh".to_string(),
    };

    state.add_user(user).await?;
    let issue_id = state
        .add_issue(Issue::new(
            IssueType::Task,
            "task".to_string(),
            username.clone(),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .await?;

    let task_id = TaskId::new();
    let task = Task::new(
        "prompt".to_string(),
        BundleSpec::None,
        Some(issue_id),
        None,
        HashMap::new(),
        None,
        None,
    );
    let (actor, auth_token) = Actor::new_for_task(task_id.clone());
    state.add_task_with_id(task_id, task, Utc::now()).await?;
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state).await?;
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
    let state = test_state();
    let task_id = TaskId::new();
    let (actor, auth_token) = Actor::new_for_task(task_id);
    state.add_actor(actor).await?;

    let server = spawn_test_server_with_state(state).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    Ok(())
}
