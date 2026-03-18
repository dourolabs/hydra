use crate::test_utils::{spawn_test_server, test_client};
use serde_json::json;

#[tokio::test]
async fn health_route_runs_with_injected_dependencies() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .get(format!("{}/health", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await?;
    assert_eq!(body, json!({ "status": "ok" }));

    Ok(())
}
