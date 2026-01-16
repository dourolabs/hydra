use crate::app::AppState;
use axum::{Json, extract::State};
use metis_common::agents::{AgentRecord, ListAgentsResponse};
use tracing::info;

pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<ListAgentsResponse>, crate::routes::jobs::ApiError> {
    info!("list_agents invoked");
    let agents = state
        .config
        .background
        .agent_queues
        .iter()
        .map(|queue| AgentRecord {
            name: queue.name.clone(),
        })
        .collect();

    Ok(Json(ListAgentsResponse { agents }))
}
