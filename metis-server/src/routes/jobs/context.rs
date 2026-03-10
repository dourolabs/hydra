use crate::{
    app::AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use metis_common::{api::v1, constants::ENV_METIS_ID};
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<v1::jobs::WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let task = state.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    let resolved = state.resolve_task(&task).await.map_err(ApiError::from)?;

    let mut env_vars = resolved.env_vars;
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_METIS_ID.to_string(), job_id.to_string());

    let build_cache = state.config.build_cache.to_context();
    let context = v1::jobs::WorkerContext::new(
        resolved.context.bundle.into(),
        task.prompt,
        task.model.clone(),
        env_vars,
        build_cache,
    );
    info!(job_id = %job_id, "get_job_context completed");
    Ok(Json(context))
}
