use crate::{
    app::{AppState, ServiceState},
    store::MemoryStore,
    test_utils::{MockJobEngine, spawn_test_server_with_state, test_app_config, test_client},
};
use metis_common::github::GithubAppClientIdResponse;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::test]
async fn github_app_client_id_returns_configured_value() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.github_app.client_id = Some("client-123".to_string());

    let state = AppState {
        config: Arc::new(config),
        github_app: None,
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine: Arc::new(MockJobEngine::new()),
        spawners: Vec::new(),
    };

    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/github/app/client-id", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GithubAppClientIdResponse = response.json().await?;
    assert_eq!(
        body,
        GithubAppClientIdResponse {
            client_id: "client-123".to_string()
        }
    );

    Ok(())
}

#[tokio::test]
async fn github_app_client_id_returns_not_found_when_unset() -> anyhow::Result<()> {
    let state = AppState {
        config: Arc::new(test_app_config()),
        github_app: None,
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine: Arc::new(MockJobEngine::new()),
        spawners: Vec::new(),
    };

    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/github/app/client-id", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    Ok(())
}
