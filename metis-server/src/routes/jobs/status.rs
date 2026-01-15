use crate::{
    app::{AppState, SetJobStatusError},
    routes::jobs::{ApiError, JobIdPath},
};
use anyhow::anyhow;
use axum::{Json, extract::State};
use metis_common::job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse};
use tracing::{error, info};

pub async fn set_job_status(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
    Json(status): Json<JobStatusUpdate>,
) -> Result<Json<SetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, status = ?status, "set_job_status invoked");

    let response = state
        .set_job_status(job_id, status)
        .await
        .map_err(|err| match err {
            SetJobStatusError::NotFound { source, job_id } => {
                error!(
                    error = %source,
                    job_id = %job_id,
                    "failed to get task for status update"
                );
                ApiError::not_found(format!("Job '{job_id}' not found in store"))
            }
            SetJobStatusError::Store { source, job_id } => {
                error!(error = %source, job_id = %job_id, "failed to update task status");
                ApiError::internal(anyhow!("Failed to update task status: {source}"))
            }
        })?;

    info!(job_id = %response.job_id, "job status stored successfully");
    Ok(Json(response))
}

pub async fn get_job_status(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<GetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, "get_job_status invoked");

    let store = state.store.read().await;
    store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to load task for job status");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    let status_log = store.get_status_log(&job_id).await.map_err(|err| {
        error!(
            error = %err,
            job_id = %job_id,
            "failed to load status log for job status"
        );
        ApiError::internal(anyhow!("Failed to load status log: {err}"))
    })?;

    Ok(Json(GetJobStatusResponse { job_id, status_log }))
}
