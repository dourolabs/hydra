use crate::{AppState, routes::jobs::ApiError, store::Task};
use anyhow::anyhow;
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::{job_outputs::JobOutputPayload, jobs::WorkerContext};
use std::collections::HashMap;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<WorkerContext>, ApiError> {
    let job_id = job_id.trim();
    info!(job_id = %job_id, "get_job_context invoked");
    if job_id.is_empty() {
        error!("get_job_context received an empty job_id");
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    let store = state.store.read().await;
    let job_id_string = job_id.to_string();
    let task = store.get_task(&job_id_string).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    match task {
        Task::Spawn { context, .. } => Ok(Json(WorkerContext {
            request_context: context.clone(),
            parents: HashMap::new(),
        })),
    }
}
