use crate::{
    domain::{
        actors::ActorRef,
        users::{User, Username},
    },
    test::{spawn_test_server_with_state, test_actor, test_client, test_state_handles},
};
use metis_common::api::v1::users::UserSummary;
use reqwest::StatusCode;

#[tokio::test]
async fn get_user_returns_user_summary() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let user = User::new(
        Username::from("testuser"),
        Some(12345),
        "gh-token".to_string(),
        "gh-refresh".to_string(),
        false,
    );
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
    let user = User::new(
        Username::from("tokenuser"),
        Some(99999),
        "secret-gh-token".to_string(),
        "secret-gh-refresh".to_string(),
        false,
    );
    handles.store.add_user(user, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/tokenuser", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: serde_json::Value = response.json().await?;

    // Verify no token fields are present in response
    assert!(body.get("github_token").is_none());
    assert!(body.get("github_refresh_token").is_none());

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
async fn get_user_me_resolves_to_authenticated_user() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let actor = test_actor();
    let username = actor.creator.clone();

    let user = User::new(
        username.clone(),
        Some(77777),
        "gh-token".to_string(),
        "gh-refresh".to_string(),
        false,
    );
    handles.store.add_user(user, &ActorRef::test()).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/me", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: UserSummary = response.json().await?;
    assert_eq!(body.username.as_str(), username.as_str());
    assert_eq!(body.github_user_id, Some(77777));

    Ok(())
}
