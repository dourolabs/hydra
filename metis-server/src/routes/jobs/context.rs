use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
    store::Task,
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

    let Task {
        program,
        params,
        context,
        env_vars,
        ..
    } = task;

    Ok(Json(WorkerContext {
        request_context: context,
        program,
        params,
        variables: env_vars,
    }))
}
