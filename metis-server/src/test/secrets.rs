use crate::{
    app::{AppState, ServiceState},
    domain::{actors::ActorRef, secrets::SecretManager, sessions::BundleSpec, users::Username},
    store::{MemoryStore, Session, Status},
    test::{
        MockJobEngine, TestStateHandles, spawn_test_server_with_state, test_app_config,
        test_client, test_client_without_auth, test_user_client,
    },
};
use chrono::Utc;
use metis_common::api::v1::{self, secrets::ListSecretsResponse};
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
    );
    TestStateHandles { state, store }
}

// The test user actor has ActorId::Username("test-creator"), so we use that as the username.
const TEST_USERNAME: &str = "test-creator";

#[tokio::test]
async fn list_secrets_empty() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_user_client();

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
    let client = test_user_client();

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
    let client = test_user_client();

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
async fn me_returns_forbidden() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_user_client();

    // "me" is no longer resolved — should be treated as a different user and return 403
    let response = client
        .get(format!("{}/v1/users/me/secrets", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn forbidden_for_other_user() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_user_client();

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
    let client = test_user_client();

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
    let client = test_user_client();

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
    let client = test_user_client();

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
    let client = test_user_client();

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

#[tokio::test]
async fn session_actor_forbidden_even_when_creator_matches() -> anyhow::Result<()> {
    let handles = test_state_with_secrets();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    // test_client() uses a session actor whose creator is "test-creator"
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/users/{TEST_USERNAME}/secrets",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

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
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &None)
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
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &None)
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
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &None)
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
    );
    let username = Username::from("dave");

    let mut env_vars = HashMap::new();
    state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &None)
        .await;

    // Nothing stored, nothing in config — system secrets should be absent
    assert!(!env_vars.contains_key("OPENAI_API_KEY"));
    assert!(!env_vars.contains_key("ANTHROPIC_API_KEY"));
    assert!(!env_vars.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
}

#[tokio::test]
async fn resolve_secrets_injects_listed_user_secrets() {
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

    let filter = Some(vec![
        "MY_CUSTOM_SECRET".to_string(),
        "ANOTHER_SECRET".to_string(),
    ]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    // Custom user secrets should be injected when listed in filter
    assert_eq!(env_vars.get("MY_CUSTOM_SECRET").unwrap(), "my-custom-value");
    assert_eq!(env_vars.get("ANOTHER_SECRET").unwrap(), "another-value");
    // System secrets should fall back to config
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "global-openai-key");
}

// ---- Task.secrets filtering tests ----

#[tokio::test]
async fn resolve_secrets_custom_secrets_not_injected_when_filter_is_none() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("filter-none");

    // Store AI key and custom secret
    let encrypted_ai = secret_manager.encrypt("user-openai").unwrap();
    handles
        .store
        .set_user_secret(&username, "OPENAI_API_KEY", &encrypted_ai)
        .await
        .unwrap();

    let encrypted_custom = secret_manager.encrypt("custom-val").unwrap();
    handles
        .store
        .set_user_secret(&username, "MY_CUSTOM_SECRET", &encrypted_custom)
        .await
        .unwrap();

    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &None)
        .await;

    // AI key should be injected
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "user-openai");
    // Custom secret should NOT be injected
    assert!(!env_vars.contains_key("MY_CUSTOM_SECRET"));
}

#[tokio::test]
async fn resolve_secrets_custom_secrets_injected_when_listed() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("filter-listed");

    let encrypted = secret_manager.encrypt("custom-val").unwrap();
    handles
        .store
        .set_user_secret(&username, "MY_CUSTOM_SECRET", &encrypted)
        .await
        .unwrap();

    let filter = Some(vec!["MY_CUSTOM_SECRET".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    assert_eq!(env_vars.get("MY_CUSTOM_SECRET").unwrap(), "custom-val");
}

#[tokio::test]
async fn resolve_secrets_ai_keys_always_injected_regardless_of_filter() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("filter-ai");

    // Store all three AI keys as user secrets
    let enc1 = secret_manager.encrypt("user-openai").unwrap();
    handles
        .store
        .set_user_secret(&username, "OPENAI_API_KEY", &enc1)
        .await
        .unwrap();

    let enc2 = secret_manager.encrypt("user-anthropic").unwrap();
    handles
        .store
        .set_user_secret(&username, "ANTHROPIC_API_KEY", &enc2)
        .await
        .unwrap();

    let enc3 = secret_manager.encrypt("user-claude-oauth").unwrap();
    handles
        .store
        .set_user_secret(&username, "CLAUDE_CODE_OAUTH_TOKEN", &enc3)
        .await
        .unwrap();

    // Empty filter — no custom secrets requested
    let filter: Option<Vec<String>> = Some(vec![]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    // All AI keys should still be injected
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "user-openai");
    assert_eq!(env_vars.get("ANTHROPIC_API_KEY").unwrap(), "user-anthropic");
    assert_eq!(
        env_vars.get("CLAUDE_CODE_OAUTH_TOKEN").unwrap(),
        "user-claude-oauth"
    );
}

#[tokio::test]
async fn resolve_secrets_mixed_filter_only_listed_custom_secrets_appear() {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();
    let username = Username::from("filter-mixed");

    // Store AI key + two custom secrets
    let enc_ai = secret_manager.encrypt("user-openai").unwrap();
    handles
        .store
        .set_user_secret(&username, "OPENAI_API_KEY", &enc_ai)
        .await
        .unwrap();

    let enc1 = secret_manager.encrypt("allowed-val").unwrap();
    handles
        .store
        .set_user_secret(&username, "ALLOWED_SECRET", &enc1)
        .await
        .unwrap();

    let enc2 = secret_manager.encrypt("blocked-val").unwrap();
    handles
        .store
        .set_user_secret(&username, "BLOCKED_SECRET", &enc2)
        .await
        .unwrap();

    // Only ALLOWED_SECRET in the filter
    let filter = Some(vec!["ALLOWED_SECRET".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    // AI key always present
    assert_eq!(env_vars.get("OPENAI_API_KEY").unwrap(), "user-openai");
    // Listed custom secret present
    assert_eq!(env_vars.get("ALLOWED_SECRET").unwrap(), "allowed-val");
    // Unlisted custom secret absent
    assert!(!env_vars.contains_key("BLOCKED_SECRET"));
}

// ---- GH_TOKEN auto-injection tests ----

/// Creates a test state with local auth mode (no GitHub App), so
/// get_github_token_for_user returns the stored token without calling GitHub APIs.
fn test_state_local_auth() -> TestStateHandles {
    use crate::config::AuthConfig;

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "unused-pat".to_string(),
        username: None,
        auth_token_file: None,
    };
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );
    TestStateHandles { state, store }
}

#[tokio::test]
async fn resolve_secrets_gh_token_injected_when_in_filter_and_user_has_github_token() {
    let handles = test_state_local_auth();
    let secret_manager = test_secret_manager();
    let username = Username::from("gh-user");

    // Store the user's GitHub OAuth token (GITHUB_TOKEN + GITHUB_REFRESH_TOKEN)
    let encrypted = secret_manager.encrypt("gho_test_github_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_TOKEN", &encrypted)
        .await
        .unwrap();
    let encrypted_refresh = secret_manager.encrypt("ghr_test_refresh_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_REFRESH_TOKEN", &encrypted_refresh)
        .await
        .unwrap();

    // Request GH_TOKEN in the secrets filter
    let filter = Some(vec!["GH_TOKEN".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    assert_eq!(
        env_vars.get("GH_TOKEN").map(String::as_str),
        Some("gho_test_github_token"),
        "GH_TOKEN should be auto-injected from creator's GitHub OAuth token"
    );
}

#[tokio::test]
async fn resolve_secrets_gh_token_not_injected_when_not_in_filter() {
    let handles = test_state_local_auth();
    let secret_manager = test_secret_manager();
    let username = Username::from("gh-no-filter");

    // Store the user's GitHub OAuth token (GITHUB_TOKEN + GITHUB_REFRESH_TOKEN)
    let encrypted = secret_manager.encrypt("gho_test_github_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_TOKEN", &encrypted)
        .await
        .unwrap();
    let encrypted_refresh = secret_manager.encrypt("ghr_test_refresh_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_REFRESH_TOKEN", &encrypted_refresh)
        .await
        .unwrap();

    // No GH_TOKEN in the secrets filter
    let filter = Some(vec!["SOME_OTHER_SECRET".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    assert!(
        !env_vars.contains_key("GH_TOKEN"),
        "GH_TOKEN should not be injected when not in secrets filter"
    );
}

#[tokio::test]
async fn resolve_secrets_gh_token_not_injected_when_user_has_no_github_token() {
    let handles = test_state_local_auth();
    let username = Username::from("gh-no-token");

    // No GITHUB_TOKEN stored for this user
    let filter = Some(vec!["GH_TOKEN".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    assert!(
        !env_vars.contains_key("GH_TOKEN"),
        "GH_TOKEN should not be injected when user has no GitHub token"
    );
}

#[tokio::test]
async fn resolve_secrets_user_set_gh_token_takes_priority_over_auto_injection() {
    let handles = test_state_local_auth();
    let secret_manager = test_secret_manager();
    let username = Username::from("gh-override");

    // Store the user's GitHub OAuth token (GITHUB_TOKEN + GITHUB_REFRESH_TOKEN)
    let encrypted_github = secret_manager.encrypt("gho_auto_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_TOKEN", &encrypted_github)
        .await
        .unwrap();
    let encrypted_refresh = secret_manager.encrypt("ghr_test_refresh_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GITHUB_REFRESH_TOKEN", &encrypted_refresh)
        .await
        .unwrap();

    // Also store a user-set GH_TOKEN secret (explicit override)
    let encrypted_gh = secret_manager.encrypt("gho_user_set_token").unwrap();
    handles
        .store
        .set_user_secret(&username, "GH_TOKEN", &encrypted_gh)
        .await
        .unwrap();

    let filter = Some(vec!["GH_TOKEN".to_string()]);
    let mut env_vars = HashMap::new();
    handles
        .state
        .resolve_secrets_into_env_vars(&username, &mut env_vars, &filter)
        .await;

    assert_eq!(
        env_vars.get("GH_TOKEN").map(String::as_str),
        Some("gho_user_set_token"),
        "User-set GH_TOKEN should take priority over auto-injected value"
    );
}

// ---- End-to-end integration test: user secret appears in get_job_context ----

#[tokio::test]
async fn get_job_context_includes_user_secrets() -> anyhow::Result<()> {
    let handles = test_state_with_secrets_and_config();
    let secret_manager = test_secret_manager();

    let creator = Username::from(TEST_USERNAME);

    // Store a user secret for CLAUDE_CODE_OAUTH_TOKEN
    let encrypted = secret_manager.encrypt("user-oauth-token-value").unwrap();
    handles
        .store
        .set_user_secret(&creator, "CLAUDE_CODE_OAUTH_TOKEN", &encrypted)
        .await
        .unwrap();

    // Create a task owned by the test creator
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
                prompt: "test prompt".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: creator.clone(),
                image: Some("test-image:latest".to_string()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(handles.state, handles.store.clone()).await?;
    let client = test_client();

    // Call get_session_context and verify secrets appear in variables
    let response = client
        .get(format!(
            "{}/v1/sessions/{job_id}/context",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: v1::sessions::WorkerContext = response.json().await?;

    // User's CLAUDE_CODE_OAUTH_TOKEN should override (config has None for this key)
    assert_eq!(
        body.variables
            .get("CLAUDE_CODE_OAUTH_TOKEN")
            .map(String::as_str),
        Some("user-oauth-token-value"),
        "user secret should appear in get_job_context response"
    );

    // Config fallback keys should also be present
    assert_eq!(
        body.variables.get("OPENAI_API_KEY").map(String::as_str),
        Some("global-openai-key"),
        "config fallback should be used when no user secret exists"
    );
    assert_eq!(
        body.variables.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("global-anthropic-key"),
        "config fallback should be used when no user secret exists"
    );

    // METIS_ID should also be set
    assert_eq!(
        body.variables.get("METIS_ID").map(String::as_str),
        Some(job_id.as_ref())
    );

    Ok(())
}
