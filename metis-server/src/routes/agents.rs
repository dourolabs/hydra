use crate::{
    app::{AgentError, AppState},
    config::AgentQueueConfig,
};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::api::v1::{
    ApiError,
    agents::{
        AgentRecord, AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest,
    },
};
use tracing::{error, info};

pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<ListAgentsResponse>, ApiError> {
    info!("list_agents invoked");
    let agents = state
        .list_agent_configs()
        .await
        .into_iter()
        .map(AgentRecord::from)
        .collect();

    let response = ListAgentsResponse::new(agents);
    info!(agent_count = response.agents.len(), "list_agents completed");
    Ok(Json(response))
}

pub async fn get_agent(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %agent_name, "get_agent invoked");
    let Some(agent) = state.get_agent_config(&agent_name).await else {
        error!(agent = %agent_name, "agent not found");
        return Err(ApiError::not_found(format!(
            "agent '{agent_name}' not found"
        )));
    };

    info!(agent = %agent_name, "get_agent completed");
    Ok(Json(AgentResponse::new(AgentRecord::from(agent))))
}

pub async fn create_agent(
    State(state): State<AppState>,
    Json(payload): Json<UpsertAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %payload.name, "create_agent invoked");
    let config = normalize_agent(payload)?;
    let created = state.create_agent(config).await.map_err(map_agent_error)?;

    info!(agent = %created.name, "create_agent completed");
    Ok(Json(AgentResponse::new(AgentRecord::from(created))))
}

pub async fn update_agent(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
    Json(payload): Json<UpsertAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %agent_name, "update_agent invoked");
    let config = normalize_agent(payload)?;
    if config.name != agent_name {
        return Err(ApiError::bad_request(
            "agent name must match path parameter".to_string(),
        ));
    }

    let updated = state
        .update_agent(&agent_name, config)
        .await
        .map_err(map_agent_error)?;

    info!(agent = %agent_name, "update_agent completed");
    Ok(Json(AgentResponse::new(AgentRecord::from(updated))))
}

pub async fn delete_agent(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
) -> Result<Json<DeleteAgentResponse>, ApiError> {
    info!(agent = %agent_name, "delete_agent invoked");
    let deleted = state
        .delete_agent(&agent_name)
        .await
        .map_err(map_agent_error)?;

    info!(agent = %agent_name, "delete_agent completed");
    Ok(Json(DeleteAgentResponse::new(AgentRecord::from(deleted))))
}

fn normalize_agent(payload: UpsertAgentRequest) -> Result<AgentQueueConfig, ApiError> {
    Ok(AgentQueueConfig {
        name: normalize_non_empty("name", payload.name)?,
        prompt: normalize_non_empty("prompt", payload.prompt)?,
        max_tries: payload.max_tries,
        max_simultaneous: payload.max_simultaneous,
        match_unassigned: payload.match_unassigned,
    })
}

fn normalize_non_empty(field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(trimmed.to_string())
}

fn map_agent_error(err: AgentError) -> ApiError {
    match err {
        AgentError::AlreadyExists { name } => {
            error!(agent = %name, "agent already exists");
            ApiError::conflict(format!("agent '{name}' already exists"))
        }
        AgentError::NotFound { name } => {
            error!(agent = %name, "agent not found");
            ApiError::not_found(format!("agent '{name}' not found"))
        }
    }
}

impl From<AgentQueueConfig> for AgentRecord {
    fn from(config: AgentQueueConfig) -> Self {
        AgentRecord::with_details(
            config.name,
            config.prompt,
            config.max_tries,
            config.max_simultaneous,
            config.match_unassigned,
        )
    }
}
