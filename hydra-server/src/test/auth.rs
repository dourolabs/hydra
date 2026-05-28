use crate::domain::actors::Actor;
use crate::domain::users::Username;
use crate::test_utils::{
    register_actor_and_token, spawn_test_server, spawn_test_server_with_state,
    test_client_without_auth, test_state_handles,
};
use hydra_common::{ActorId, SessionId};
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
async fn protected_routes_require_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn public_routes_accept_requests_without_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!("{}/health", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let response = client
        .get(format!("{}/v1/github/app/client-id", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

/// A token row that has been flipped to `is_revoked = true` must be rejected
/// by `require_auth` exactly like an unknown token — a 401 with the
/// `authorization invalid` body.
#[tokio::test]
async fn revoked_session_token_returns_401() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let session_id = SessionId::new();
    let (actor, auth_token) = Actor::new_from_actor_id(
        ActorId::Adhoc(session_id.clone()),
        Username::from("creator"),
        None,
    );
    register_actor_and_token(
        handles.store.as_ref(),
        &actor,
        &auth_token,
        Some(&session_id),
    )
    .await?;

    let store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(&auth_token);

    // Before revocation: token authenticates fine.
    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    store.revoke_auth_tokens_for_session(&session_id).await?;

    let response = client
        .get(format!("{}/v1/whoami", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.text().await?;
    assert!(
        body.contains("authorization invalid"),
        "revoked token should be rejected with 'authorization invalid', got: {body}"
    );

    Ok(())
}
