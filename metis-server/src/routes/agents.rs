use crate::app::AppState;
use axum::{Json, extract::State};
use metis_common::api::v1::{
    agents::{AgentRecord, ListAgentsResponse},
    ApiError,
};
use tracing::info;

pub async fn list_agents(State(state): State<AppState>) -> Result<Json<ListAgentsResponse>, ApiError> {
    info!("list_agents invoked");
    let agents = state
        .config
        .background
        .agent_queues
        .iter()
        .map(|queue| AgentRecord::new(queue.name.clone()))
        .collect();

    let response = ListAgentsResponse::new(agents);
    info!(agent_count = response.agents.len(), "list_agents completed");
    Ok(Json(response))
}
