use crate::{
    app::{AppState, GitCache},
    store::MemoryStore,
    test_utils::{MockJobEngine, spawn_test_server_with_state, test_app_config, test_client},
};
use metis_common::github::GithubAppClientIdResponse;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::test]
async fn github_app_client_id_returns_configured_value() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.github_app.client_id = "client-123".to_string();

    let state = AppState {
        config: Arc::new(config),
        github_app: None,
        git_cache: Arc::new(GitCache::default()),
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
