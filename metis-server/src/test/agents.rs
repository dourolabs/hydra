use crate::{
    app::{AppState, ServiceState},
    background::AgentQueue,
    config::{AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
    store::MemoryStore,
    test_utils::{MockJobEngine, spawn_test_server_with_state, test_app_config, test_client},
};
use metis_common::agents::{
    AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest,
};
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_state_with_agents(agent_names: &[&str]) -> AppState {
    let mut config = test_app_config();
    let agents: Vec<AgentQueueConfig> = agent_names
        .iter()
        .map(|name| AgentQueueConfig {
            name: (*name).to_string(),
            prompt: format!("prompt for {name}"),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
        })
        .collect();
    config.background.agent_queues = agents.clone();

    AppState {
        config: Arc::new(config),
        github_app: None,
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine: Arc::new(MockJobEngine::new()),
        agents: Arc::new(RwLock::new(
            agents
                .iter()
                .map(|queue| Arc::new(AgentQueue::from_config(queue)))
                .collect(),
        )),
    }
}

fn agent_request(name: &str) -> UpsertAgentRequest {
    UpsertAgentRequest::new(name, format!("prompt for {name}"))
        .with_limits(DEFAULT_AGENT_MAX_TRIES, DEFAULT_AGENT_MAX_SIMULTANEOUS)
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
    let names: Vec<String> = body.agents.iter().map(|agent| agent.name.clone()).collect();

    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    assert_eq!(body.agents[0].prompt, "prompt for alpha");
    assert_eq!(
        body.agents[1].max_simultaneous,
        DEFAULT_AGENT_MAX_SIMULTANEOUS
    );
    Ok(())
}

#[tokio::test]
async fn get_agent_returns_single_queue() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]);
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/agents/alpha", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: AgentResponse = response.json().await?;
    assert_eq!(body.agent.name, "alpha");
    assert_eq!(body.agent.prompt, "prompt for alpha");
    assert_eq!(body.agent.max_tries, DEFAULT_AGENT_MAX_TRIES);
    Ok(())
}

#[tokio::test]
async fn create_agent_adds_to_state() -> anyhow::Result<()> {
    let state = test_state_with_agents(&[]);
    let server = spawn_test_server_with_state(state.clone()).await?;
    let client = test_client();

    let request = agent_request("gamma");
    let response = client
        .post(format!("{}/v1/agents", server.base_url()))
        .json(&request)
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: AgentResponse = response.json().await?;
    assert_eq!(body.agent.name, "gamma");

    let agents = state.list_agent_configs().await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "gamma");
    assert_eq!(agents[0].prompt, "prompt for gamma");
    Ok(())
}

#[tokio::test]
async fn update_agent_modifies_existing_queue() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]);
    let server = spawn_test_server_with_state(state.clone()).await?;
    let client = test_client();

    let request = UpsertAgentRequest::new("alpha", "updated prompt").with_limits(7, 11);
    let response = client
        .put(format!("{}/v1/agents/alpha", server.base_url()))
        .json(&request)
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: AgentResponse = response.json().await?;
    assert_eq!(body.agent.prompt, "updated prompt");
    assert_eq!(body.agent.max_tries, 7);
    assert_eq!(body.agent.max_simultaneous, 11);

    let agents = state.list_agent_configs().await;
    assert_eq!(agents[0].prompt, "updated prompt");
    assert_eq!(agents[0].max_tries, 7);
    assert_eq!(agents[0].max_simultaneous, 11);
    Ok(())
}

#[tokio::test]
async fn delete_agent_removes_queue() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]);
    let server = spawn_test_server_with_state(state.clone()).await?;
    let client = test_client();

    let response = client
        .delete(format!("{}/v1/agents/alpha", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: DeleteAgentResponse = response.json().await?;
    assert_eq!(body.agent.name, "alpha");

    let agents = state.list_agent_configs().await;
    assert!(agents.is_empty());
    Ok(())
}

#[tokio::test]
async fn update_agent_rejects_name_mismatch() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]);
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let request = agent_request("beta");
    let response = client
        .put(format!("{}/v1/agents/alpha", server.base_url()))
        .json(&request)
        .send()
        .await?;

    assert_eq!(response.status(), 400);
    Ok(())
}
