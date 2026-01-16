use crate::{
    app::{AppState, ServiceState},
    config::{AgentQueueConfig, DEFAULT_AGENT_MAX_TRIES},
    job_engine::MockJobEngine,
    store::MemoryStore,
    test::{spawn_test_server_with_state, test_app_config, test_client},
};
use metis_common::{agents::ListAgentsResponse, jobs::BundleSpec};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

fn test_state_with_agents(agent_names: &[&str]) -> AppState {
    let mut config = test_app_config();
    config.background.agent_queues = agent_names
        .iter()
        .map(|name| AgentQueueConfig {
            name: (*name).to_string(),
            prompt: format!("prompt for {name}"),
            context: BundleSpec::None,
            image: None,
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            env_vars: HashMap::new(),
        })
        .collect();

    AppState {
        config: Arc::new(config),
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine: Arc::new(MockJobEngine::new()),
        spawners: Vec::new(),
    }
}

#[tokio::test]
async fn list_agents_returns_configured_queues() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha", "beta"]);
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/agents", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: ListAgentsResponse = response.json().await?;
    let names: Vec<String> = body.agents.into_iter().map(|agent| agent.name).collect();

    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    Ok(())
}
