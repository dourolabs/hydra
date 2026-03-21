use crate::{
    app::AppState,
    background::spawner::AGENT_NAME_ENV_VAR,
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{Json, extract::State};
use hydra_common::{api::v1, constants::ENV_HYDRA_ID};
use tracing::{error, info, warn};

pub async fn get_session_context(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::WorkerContext>, ApiError> {
    info!(session_id = %session_id, "get_session_context invoked");

    let task = state.get_session(&session_id).await.map_err(|err| {
        error!(error = %err, session_id = %session_id, "failed to get task");
        ApiError::not_found(format!("Session '{session_id}' not found"))
    })?;

    let resolved = state.resolve_task(&task).await.map_err(ApiError::from)?;

    let mut env_vars = resolved.env_vars;
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_HYDRA_ID.to_string(), session_id.to_string());

    // Look up MCP config document for this agent (convention: agents/<name>/mcp-config.json).
    let mcp_config = if let Some(agent_name) = env_vars.get(AGENT_NAME_ENV_VAR) {
        let mcp_path = format!("agents/{agent_name}/mcp-config.json");
        match state.get_documents_by_path(&mcp_path).await {
            Ok(docs) => docs
                .into_iter()
                .next()
                .map(|(_, doc)| doc.item.body_markdown),
            Err(err) => {
                warn!(
                    session_id = %session_id,
                    agent_name = agent_name,
                    error = %err,
                    "failed to look up MCP config document"
                );
                None
            }
        }
    } else {
        None
    };

    let build_cache = state.config.build_cache.to_context();
    let context = v1::sessions::WorkerContext::new(
        resolved.context.bundle.into(),
        task.prompt,
        task.model.clone(),
        env_vars,
        build_cache,
        mcp_config,
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
