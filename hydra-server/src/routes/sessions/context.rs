use crate::{
    app::AppState,
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{Json, extract::State};
use hydra_common::{api::v1, constants::ENV_HYDRA_ID};
use tracing::{error, info};

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

    let build_cache = state.config.build_cache.to_context();
    let context = v1::sessions::WorkerContext::new(
        resolved.context.bundle.into(),
        task.prompt,
        task.model.clone(),
        env_vars,
        build_cache,
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
