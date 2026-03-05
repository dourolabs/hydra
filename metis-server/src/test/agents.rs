use crate::{
    app::{AppState, ServiceState},
    config::DEFAULT_AGENT_MAX_SIMULTANEOUS,
    domain::{actors::ActorRef, agents::Agent},
    store::{MemoryStore, Store},
    test_utils::{
        MockJobEngine, TestStateHandles, spawn_test_server_with_state, test_app_config, test_client,
    },
};
use metis_common::agents::{
    AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest,
};
use std::sync::Arc;

async fn test_state_with_agents(agent_names: &[&str]) -> TestStateHandles {
    let config = test_app_config();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
    );

    for name in agent_names {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            3,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            false,
        );
        store.add_agent(agent).await.unwrap();

        let doc = crate::domain::documents::Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        store.add_document(doc, &ActorRef::test()).await.unwrap();
    }

    TestStateHandles { state, store }
}

fn agent_request(name: &str) -> UpsertAgentRequest {
    UpsertAgentRequest::new(
        name,
        format!("prompt for {name}"),
        3,
        DEFAULT_AGENT_MAX_SIMULTANEOUS,
    )
}

#[tokio::test]
async fn list_agents_returns_configured_queues() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha", "beta"]).await;
    let server = spawn_test_server_with_state(state.state, state.store).await?;
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
    let state = test_state_with_agents(&["alpha"]).await;
    let server = spawn_test_server_with_state(state.state, state.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/agents/alpha", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: AgentResponse = response.json().await?;
    assert_eq!(body.agent.name, "alpha");
    assert_eq!(body.agent.prompt, "prompt for alpha");
    assert_eq!(body.agent.max_tries, 3);
    Ok(())
}

#[tokio::test]
async fn create_agent_adds_to_state() -> anyhow::Result<()> {
    let state = test_state_with_agents(&[]).await;
    let server = spawn_test_server_with_state(state.state.clone(), state.store.clone()).await?;
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

    let agents = state.state.list_agents().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "gamma");
    Ok(())
}

#[tokio::test]
async fn update_agent_modifies_existing_queue() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]).await;
    let server = spawn_test_server_with_state(state.state.clone(), state.store.clone()).await?;
    let client = test_client();

    let request = UpsertAgentRequest::new("alpha", "updated prompt", 7, 11);
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

    let agents = state.state.list_agents().await.unwrap();
    assert_eq!(agents[0].max_tries, 7);
    assert_eq!(agents[0].max_simultaneous, 11);
    Ok(())
}

#[tokio::test]
async fn delete_agent_removes_queue() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]).await;
    let server = spawn_test_server_with_state(state.state.clone(), state.store.clone()).await?;
    let client = test_client();

    let response = client
        .delete(format!("{}/v1/agents/alpha", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: DeleteAgentResponse = response.json().await?;
    assert_eq!(body.agent.name, "alpha");

    let agents = state.state.list_agents().await.unwrap();
    assert!(agents.is_empty());
    Ok(())
}

#[tokio::test]
async fn update_agent_rejects_name_mismatch() -> anyhow::Result<()> {
    let state = test_state_with_agents(&["alpha"]).await;
    let server = spawn_test_server_with_state(state.state, state.store).await?;
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
