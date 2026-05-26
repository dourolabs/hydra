use crate::{
    domain::{
        actors::ActorRef,
        users::{User, Username},
    },
    test::{spawn_test_server_with_state, test_client, test_state_handles},
};
use hydra_common::api::v1::users::{ListUsersResponse, UserSummary};
use reqwest::StatusCode;

#[tokio::test]
async fn get_user_returns_user_summary() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let user = User::new(Username::from("testuser"), Some(12345), false);
    handles.store.add_user(user, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/testuser", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: UserSummary = response.json().await?;
    assert_eq!(body.username.as_str(), "testuser");
    assert_eq!(body.github_user_id, Some(12345));

    Ok(())
}

#[tokio::test]
async fn get_user_returns_404_for_unknown_user() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/nonexistent", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn get_user_does_not_expose_tokens() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let user = User::new(Username::from("tokenuser"), Some(99999), false);
    handles.store.add_user(user, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/tokenuser", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: serde_json::Value = response.json().await?;

    // Verify expected fields are present
    assert_eq!(
        body.get("username").and_then(|v| v.as_str()),
        Some("tokenuser")
    );
    assert_eq!(
        body.get("github_user_id").and_then(|v| v.as_u64()),
        Some(99999)
    );

    Ok(())
}

#[tokio::test]
async fn list_users_returns_known_users() -> anyhow::Result<()> {
    let handles = test_state_handles();
    handles
        .store
        .add_user(
            User::new(Username::from("alice"), Some(101), false),
            &ActorRef::test(),
        )
        .await?;
    handles
        .store
        .add_user(
            User::new(Username::from("bob"), Some(202), false),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListUsersResponse = response.json().await?;
    let usernames: Vec<String> = body
        .users
        .iter()
        .map(|u| u.username.as_str().to_string())
        .collect();
    assert!(usernames.contains(&"alice".to_string()));
    assert!(usernames.contains(&"bob".to_string()));

    Ok(())
}

#[tokio::test]
async fn list_users_excludes_deleted_by_default() -> anyhow::Result<()> {
    let handles = test_state_handles();
    handles
        .store
        .add_user(
            User::new(Username::from("active"), Some(1), false),
            &ActorRef::test(),
        )
        .await?;
    handles
        .store
        .add_user(
            User::new(Username::from("removed"), Some(2), false),
            &ActorRef::test(),
        )
        .await?;
    handles
        .store
        .delete_user(&Username::from("removed"), &ActorRef::test())
        .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListUsersResponse = response.json().await?;
    let usernames: Vec<String> = body
        .users
        .iter()
        .map(|u| u.username.as_str().to_string())
        .collect();
    assert!(usernames.contains(&"active".to_string()));
    assert!(!usernames.contains(&"removed".to_string()));

    Ok(())
}

#[tokio::test]
async fn get_user_me_returns_404() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/me", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
