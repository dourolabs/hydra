use crate::{
    app::{AgentError, AppState},
    domain::{
        actors::{Actor, ActorRef},
        agents::Agent,
        documents::Document,
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use metis_common::api::v1::{
    ApiError,
    agents::{
        AgentRecord, AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest,
    },
    documents::SearchDocumentsQuery,
};
use tracing::{error, info};

fn default_prompt_path(name: &str) -> String {
    format!("/agents/{name}/prompt.md")
}

pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<ListAgentsResponse>, ApiError> {
    info!("list_agents invoked");
    let agents = state.list_agents().await.map_err(map_agent_error)?;

    let prompt_map = state.resolve_agent_prompts(&agents).await;
    let records: Vec<AgentRecord> = agents
        .into_iter()
        .map(|agent| {
            let prompt = prompt_map.get(&agent.name).cloned().unwrap_or_default();
            agent_to_record(agent, prompt)
        })
        .collect();

    let response = ListAgentsResponse::new(records);
    info!(agent_count = response.agents.len(), "list_agents completed");
    Ok(Json(response))
}

pub async fn get_agent(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %agent_name, "get_agent invoked");
    let agent = state
        .get_agent(&agent_name)
        .await
        .map_err(map_agent_error)?;

    let prompt = state
        .resolve_agent_prompt(&agent.prompt_path)
        .await
        .unwrap_or_default();

    info!(agent = %agent_name, "get_agent completed");
    Ok(Json(AgentResponse::new(agent_to_record(agent, prompt))))
}

pub async fn create_agent(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<UpsertAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %payload.name, "create_agent invoked");
    let (agent, prompt_text) = normalize_and_build_agent(payload)?;

    let created = state.create_agent(agent).await.map_err(map_agent_error)?;

    if let Some(prompt) = &prompt_text {
        write_prompt(&state, &created.prompt_path, prompt, &actor).await?;
    }

    info!(agent = %created.name, "create_agent completed");
    Ok(Json(AgentResponse::new(agent_to_record(
        created,
        prompt_text.unwrap_or_default(),
    ))))
}

pub async fn update_agent(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(agent_name): Path<String>,
    Json(payload): Json<UpsertAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    info!(agent = %agent_name, "update_agent invoked");
    let (agent, prompt_text) = normalize_and_build_agent(payload)?;
    if agent.name != agent_name {
        return Err(ApiError::bad_request(
            "agent name must match path parameter".to_string(),
        ));
    }

    let updated = state
        .update_agent(&agent_name, agent)
        .await
        .map_err(map_agent_error)?;

    if let Some(prompt) = &prompt_text {
        write_prompt(&state, &updated.prompt_path, prompt, &actor).await?;
    }

    let resolved_prompt = if prompt_text.is_some() {
        prompt_text.unwrap_or_default()
    } else {
        state
            .resolve_agent_prompt(&updated.prompt_path)
            .await
            .unwrap_or_default()
    };

    info!(agent = %agent_name, "update_agent completed");
    Ok(Json(AgentResponse::new(agent_to_record(
        updated,
        resolved_prompt,
    ))))
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
    Ok(Json(DeleteAgentResponse::new(agent_to_record(
        deleted,
        String::new(),
    ))))
}

fn normalize_and_build_agent(
    payload: UpsertAgentRequest,
) -> Result<(Agent, Option<String>), ApiError> {
    let name = normalize_non_empty("name", payload.name)?;
    let prompt_path = if payload.prompt_path.trim().is_empty() {
        default_prompt_path(&name)
    } else {
        payload.prompt_path.trim().to_string()
    };

    let prompt_text = if payload.prompt.trim().is_empty() {
        None
    } else {
        Some(payload.prompt.trim().to_string())
    };

    let agent = Agent::new(
        name,
        prompt_path,
        payload.max_tries,
        payload.max_simultaneous,
        payload.is_assignment_agent,
        payload.secrets,
    );

    Ok((agent, prompt_text))
}

fn normalize_non_empty(field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn agent_to_record(agent: Agent, prompt: String) -> AgentRecord {
    AgentRecord::new(
        agent.name,
        prompt,
        agent.prompt_path,
        agent.max_tries,
        agent.max_simultaneous,
        agent.is_assignment_agent,
        agent.secrets,
    )
}

async fn write_prompt(
    state: &AppState,
    prompt_path: &str,
    prompt: &str,
    actor: &Actor,
) -> Result<(), ApiError> {
    let query =
        SearchDocumentsQuery::new(None, Some(prompt_path.to_string()), Some(true), None, None);

    let existing = state
        .list_documents(&query)
        .await
        .map_err(|e| ApiError::internal(format!("failed to query document store: {e}")))?;

    let document = Document {
        title: format!("Agent prompt: {prompt_path}"),
        body_markdown: prompt.to_string(),
        path: Some(prompt_path.parse().map_err(|e| {
            ApiError::bad_request(format!("invalid prompt_path '{prompt_path}': {e}"))
        })?),
        created_by: None,
        deleted: false,
    };

    let document_id = existing.into_iter().next().map(|(id, _)| id);

    state
        .upsert_document(document_id, document, ActorRef::from(actor))
        .await
        .map_err(|e| {
            ApiError::internal(format!("failed to write prompt to document store: {e}"))
        })?;

    Ok(())
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
        AgentError::AssignmentAgentConflict => {
            error!("assignment agent conflict");
            ApiError::conflict("only one assignment agent is allowed".to_string())
        }
        AgentError::Store(err) => {
            error!(error = %err, "agent store error");
            ApiError::internal(format!("store error: {err}"))
        }
    }
}
