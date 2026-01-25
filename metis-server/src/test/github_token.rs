use crate::{
    domain::{
        actors::Actor,
        issues::{Issue, IssueStatus, IssueType},
        jobs::{BundleSpec, Task},
        users::{User, Username},
    },
    test_utils::{spawn_test_server_with_state, test_client_without_auth, test_state},
};
use chrono::Utc;
use httpmock::prelude::*;
use metis_common::{TaskId, github::GithubTokenResponse};
use octocrab::Octocrab;
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
    let (user, actor, auth_token) =
        Actor::new_for_github_token_with_client("gh-token".to_string(), &github_client).await?;

    let state = test_state();
    {
        let mut store = state.store.write().await;
        store.add_user(user).await?;
        store.add_actor(actor).await?;
    }

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
    let state = test_state();
    let username = Username::from("creator");
    let user = User::new(username.clone(), "task-token".to_string());

    let issue_id = {
        let mut store = state.store.write().await;
        store.add_user(user).await?;
        store
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
            .await?
    };

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
    {
        let mut store = state.store.write().await;
        store.add_task_with_id(task_id, task, Utc::now()).await?;
        store.add_actor(actor).await?;
    }

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
    let (_user, actor, auth_token) =
        Actor::new_for_github_token_with_client("gh-token".to_string(), &github_client).await?;

    let state = test_state();
    {
        let mut store = state.store.write().await;
        store.add_actor(actor).await?;
    }

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
async fn github_token_returns_not_found_for_missing_task() -> anyhow::Result<()> {
    let state = test_state();
    let task_id = TaskId::new();
    let (actor, auth_token) = Actor::new_for_task(task_id);
    {
        let mut store = state.store.write().await;
        store.add_actor(actor).await?;
    }

    let server = spawn_test_server_with_state(state).await?;
    let client = auth_client(&auth_token);
    let response = client
        .get(format!("{}/v1/github/token", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    Ok(())
}
