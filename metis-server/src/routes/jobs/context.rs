use crate::{
    app::{AppState, TaskExt},
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use metis_common::jobs::WorkerContext;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let store = state.store.read().await;
    let task = store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    let resolved = task
        .resolve_context(state.service_state.as_ref())
        .await
        .map_err(ApiError::from)?;
    let env_vars = task.resolve_env_vars(&resolved);

    let context = WorkerContext {
        request_context: resolved.bundle,
        prompt: task.prompt,
        variables: env_vars,
    };
    info!(job_id = %job_id, "get_job_context completed");
    Ok(Json(context))
}
