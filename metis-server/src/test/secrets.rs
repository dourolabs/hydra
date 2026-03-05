use crate::{
    app::{AppState, ServiceState},
    domain::secrets::SecretManager,
    store::MemoryStore,
    test::{
        MockJobEngine, TestStateHandles, spawn_test_server_with_state, test_app_config,
        test_client, test_client_without_auth,
    },
};
use metis_common::api::v1::secrets::ListSecretsResponse;
use reqwest::StatusCode;
use serde_json::json;
use std::sync::Arc;

fn test_secret_manager() -> Arc<SecretManager> {
    Arc::new(SecretManager::new([42u8; 32]))
}

fn test_state_with_secrets() -> TestStateHandles {
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(test_app_config()),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        Some(test_secret_manager()),
    );
    TestStateHandles { state, store }
}

// The test actor has creator "test-creator", so we use that as the username.
const TEST_USERNAME: &str = "test-creator";

#[tokio::test]
async fn list_secrets_empty() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListSecretsResponse = response.json().await?;
    assert!(body.secrets.is_empty());

    Ok(())
}

#[tokio::test]
async fn set_and_list_secrets() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Set a secret
    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/OPENAI_API_KEY",
            server.base_url()
        ))
        .json(&json!({ "value": "sk-test123" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // List secrets
    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: ListSecretsResponse = response.json().await?;
    assert_eq!(body.secrets, vec!["OPENAI_API_KEY"]);

    Ok(())
}

#[tokio::test]
async fn delete_secret() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Set then delete
    client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/OPENAI_API_KEY",
            server.base_url()
        ))
        .json(&json!({ "value": "sk-test123" }))
        .send()
        .await?;

    let response = client
        .delete(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/OPENAI_API_KEY",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Verify deleted
    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    let body: ListSecretsResponse = response.json().await?;
    assert!(body.secrets.is_empty());

    Ok(())
}

#[tokio::test]
async fn me_resolves_to_current_user() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Set via "me"
    let response = client
        .put(format!(
            "{}/v1/users/me/secrets/ANTHROPIC_API_KEY",
            server.base_url()
        ))
        .json(&json!({ "value": "sk-ant-test" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // List via explicit username
    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    let body: ListSecretsResponse = response.json().await?;
    assert_eq!(body.secrets, vec!["ANTHROPIC_API_KEY"]);

    Ok(())
}

#[tokio::test]
async fn forbidden_for_other_user() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/users/other-user/secrets", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn bad_request_for_invalid_secret_name() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/INVALID_NAME",
            server.base_url()
        ))
        .json(&json!({ "value": "something" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn service_unavailable_without_secret_manager() -> anyhow::Result<()> {
    // Use default test state (no secret manager)
    let handles = crate::test::test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    Ok(())
}

#[tokio::test]
async fn unauthorized_without_auth() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn set_overwrites_existing_secret() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Set twice
    client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/OPENAI_API_KEY",
            server.base_url()
        ))
        .json(&json!({ "value": "first" }))
        .send()
        .await?;

    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/OPENAI_API_KEY",
            server.base_url()
        ))
        .json(&json!({ "value": "second" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Should still only appear once in list
    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    let body: ListSecretsResponse = response.json().await?;
    assert_eq!(body.secrets.len(), 1);

    Ok(())
}
