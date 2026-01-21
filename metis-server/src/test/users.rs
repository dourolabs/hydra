use crate::{
    domain::users::{CreateUserRequest, UpdateGithubTokenRequest, User, Username},
    test::{spawn_test_server_with_state, test_client, test_state},
};
use reqwest::StatusCode;
use serde_json::Value;

#[tokio::test]
async fn list_users_does_not_return_tokens() -> anyhow::Result<()> {
    let state = test_state();
    {
        let mut store = state.store.write().await;
        store
            .add_user(User {
                username: Username::from("alice"),
                github_token: "token-123".to_string(),
            })
            .await
            .unwrap();
    }

    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = response.json().await?;
    let users = body["users"].as_array().expect("users should be an array");
    assert_eq!(users.len(), 1);
    let user = users[0].as_object().expect("user should be an object");
    assert_eq!(user.get("username").and_then(Value::as_str), Some("alice"));
    assert!(user.get("github_token").is_none());

    Ok(())
}

#[tokio::test]
async fn set_github_token_overwrites_existing() -> anyhow::Result<()> {
    let state = test_state();
    let store = state.store.clone();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = CreateUserRequest {
        username: Username::from("bob"),
        github_token: "old-token".to_string(),
    };
    let create_response = client
        .post(format!("{}/v1/users", server.base_url()))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(create_response.status(), StatusCode::OK);

    let update_payload = UpdateGithubTokenRequest {
        github_token: "new-token".to_string(),
    };
    let update_response = client
        .put(format!("{}/v1/users/bob/github-token", server.base_url()))
        .json(&update_payload)
        .send()
        .await?;
    assert_eq!(update_response.status(), StatusCode::OK);

    let store_read = store.read().await;
    let users = store_read.list_users().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].github_token, "new-token");

    Ok(())
}

#[tokio::test]
async fn delete_missing_user_returns_not_found() -> anyhow::Result<()> {
    let state = test_state();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let response = client
        .delete(format!("{}/v1/users/missing", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
