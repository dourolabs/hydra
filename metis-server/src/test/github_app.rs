use crate::{
    app::{AppState, ServiceState},
    store::MemoryStore,
    test_utils::{MockJobEngine, spawn_test_server_with_state, test_app_config, test_client},
};
use metis_common::github::GithubAppClientIdResponse;
use std::sync::Arc;

#[tokio::test]
async fn github_app_client_id_returns_configured_value() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.github_app.client_id = "client-123".to_string();

    let store = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        None,
    );

    let server = spawn_test_server_with_state(state, store).await?;
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
