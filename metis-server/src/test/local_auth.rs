use crate::{
    app::{AppState, ServiceState},
    config::AuthConfig,
    domain::users::Username,
    routes::local_auth::LocalAuthResponse,
    setup_local_auth,
    store::{MemoryStore, ReadOnlyStore},
    test_utils::{
        MockJobEngine, spawn_test_server_with_state, test_app_config, test_client_without_auth,
        test_secret_manager,
    },
};
use reqwest::StatusCode;
use std::sync::Arc;

#[tokio::test]
async fn setup_local_auth_creates_actor() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // Actor should exist in the store.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert_eq!(actor.item.name(), "u-local");

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_is_idempotent() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());

    // Run twice — second call should not fail.
    setup_local_auth(&config, store.as_ref()).await?;
    setup_local_auth(&config, store.as_ref()).await?;

    // The actor in the store should still exist.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert_eq!(actor.item.name(), "u-local");

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_stores_github_pat() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_pat_token_123".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // User should exist.
    let username = Username::from("local");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.username, username);

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_uses_custom_username() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: Some("alice".to_string()),
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // Actor should exist under the custom username.
    let actor = store.as_ref().get_actor("u-alice").await?;
    assert_eq!(actor.item.name(), "u-alice");

    // User should also be stored under the custom username.
    let username = Username::from("alice");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.username, username);

    Ok(())
}

#[tokio::test]
async fn local_auth_endpoint_returns_token() -> anyhow::Result<()> {
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(test_app_config()),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
        Some("test-local-token-123".to_string()),
    );

    let server = spawn_test_server_with_state(state, store).await?;
    let client = test_client_without_auth();
    let response = client
        .get(format!("{}/v1/local-auth", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: LocalAuthResponse = response.json().await?;
    assert_eq!(body.token, "test-local-token-123");

    Ok(())
}

#[tokio::test]
async fn local_auth_endpoint_returns_400_when_not_local() -> anyhow::Result<()> {
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(test_app_config()),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
        None,
    );

    let server = spawn_test_server_with_state(state, store).await?;
    let client = test_client_without_auth();
    let response = client
        .get(format!("{}/v1/local-auth", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
