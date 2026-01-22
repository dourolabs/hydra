use crate::{
    app::{AppState, TaskExt},
    domain::jobs::WorkerContext,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use metis_common::api::v1;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<v1::jobs::WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let task = {
        let store = state.store.read().await;
        store.get_task(&job_id).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task");
            ApiError::not_found(format!("Job '{job_id}' not found"))
        })?
    };
    let task = state
        .apply_job_settings_to_task(task)
        .await
        .map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to apply job settings");
            ApiError::internal("failed to load job context".to_string())
        })?;

    let resolved = task
        .resolve_context(state.service_state.as_ref())
        .await
        .map_err(ApiError::from)?;
    let env_vars = task.resolve_env_vars(&resolved);

    let context: v1::jobs::WorkerContext =
        WorkerContext::new(resolved.bundle, task.prompt, env_vars).into();
    info!(job_id = %job_id, "get_job_context completed");
    Ok(Json(context))
}
