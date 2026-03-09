use crate::{
    app::{AppState, ServiceState},
    domain::secrets::SecretManager,
    domain::users::Username,
    store::MemoryStore,
    test::{
        MockJobEngine, TestStateHandles, spawn_test_server_with_state, test_app_config,
        test_client, test_client_without_auth,
    },
};
use metis_common::api::v1::secrets::ListSecretsResponse;
use reqwest::StatusCode;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

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
        test_secret_manager(),
        None,
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

    // Lowercase name is invalid
    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/invalid_name",
            server.base_url()
        ))
        .json(&json!({ "value": "something" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn bad_request_for_metis_prefix() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/METIS_TOKEN",
            server.base_url()
        ))
        .json(&json!({ "value": "something" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn set_custom_secret_name() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Set a custom secret (not one of the well-known names)
    let response = client
        .put(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets/MY_CUSTOM_SECRET",
            server.base_url()
        ))
        .json(&json!({ "value": "custom-value" }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it appears in the list
    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    let body: ListSecretsResponse = response.json().await?;
    assert!(body.secrets.contains(&"MY_CUSTOM_SECRET".to_string()));

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

// ---- resolve_secrets_into_env_vars tests ----

fn test_state_with_secrets_and_config() -> TestStateHandles {
    let mut config = test_app_config();
    config.metis.openai_api_key = Some("global-openai-key".to_string());
    config.metis.anthropic_api_key = Some("global-anthropic-key".to_string());
    // Leave claude_code_oauth_token as None to test the "neither" case

    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
        None,
    );
    TestStateHandles { state, store }
}

#[tokio::test]
async fn resolve_secrets_user_secret_takes_priority() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("alice");

    // Store an encrypted user secret for OPENAI_API_KEY
    let encrypted = secret_manager.encrypt("user-openai-key").unwrap();
    handles
        .store
        .set_user_secret(&username, "OPENAI_API_KEY", &encrypted)
        .await
        .unwrap();

    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars)
        .await;

    // User secret should take priority over config
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "user-openai-key");
    // ANTHROPIC_API_KEY should fall back to config
    assert_eq!(
        env_vars.get("ANTHROPIC_API_KEY").unwrap(),
        "global-anthropic-key"
    );
}

#[tokio::test]
async fn resolve_secrets_falls_back_to_config() {
    let handles = test_state_with_secrets_and_config();
    let username = Username::from("bob");

    // No user secrets stored — should use config values
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars)
        .await;

    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "global-openai-key");
    assert_eq!(
        env_vars.get("ANTHROPIC_API_KEY").unwrap(),
        "global-anthropic-key"
    );
    // CLAUDE_CODE_OAUTH_TOKEN has no config value, so it should be absent
    assert!(!env_vars.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
}

#[tokio::test]
async fn resolve_secrets_decryption_failure_falls_back_to_config() {
    let handles = test_state_with_secrets_and_config();
    let username = Username::from("carol");

    // Encrypt with a different key than the one in AppState
    let wrong_key_manager = Arc::new(SecretManager::new([99u8; 32]));
    let bad_encrypted = wrong_key_manager.encrypt("wrong-key-secret").unwrap();
    handles
        .store
        .set_user_secret(&username, "OPENAI_API_KEY", &bad_encrypted)
        .await
        .unwrap();

    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars)
        .await;

    // Decryption fails, so it should fall back to the global config value
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "global-openai-key");
}

#[tokio::test]
async fn resolve_secrets_no_user_secret_no_config_not_set() {
    // Config with no API keys set
    let config = test_app_config();
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
        None,
    );
    let username = Username::from("dave");

    let mut env_vars = HashMap::new();
    state
        .resolve_secrets_into_env_vars(&username, &mut env_vars)
        .await;

    // Nothing stored, nothing in config — system secrets should be absent
    assert!(!env_vars.contains_key("OPENAI_API_KEY"));
    assert!(!env_vars.contains_key("ANTHROPIC_API_KEY"));
    assert!(!env_vars.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
}

#[tokio::test]
async fn resolve_secrets_injects_all_user_secrets() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("eve");

    // Store custom user secrets
    let encrypted1 = secret_manager.encrypt("my-custom-value").unwrap();
    handles
        .store
        .set_user_secret(&username, "MY_CUSTOM_SECRET", &encrypted1)
        .await
        .unwrap();

    let encrypted2 = secret_manager.encrypt("another-value").unwrap();
    handles
        .store
        .set_user_secret(&username, "ANOTHER_SECRET", &encrypted2)
        .await
        .unwrap();

    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars)
        .await;

    // Custom user secrets should be injected
    assert_eq!(env_vars.get("MY_CUSTOM_SECRET").unwrap(), "my-custom-value");
    assert_eq!(env_vars.get("ANOTHER_SECRET").unwrap(), "another-value");
    // System secrets should fall back to config
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "global-openai-key");
}
