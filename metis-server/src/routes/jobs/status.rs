use crate::app::event_bus::with_actor;
use crate::domain::actors::Actor;
use crate::{
    app::{AppState, SetJobStatusError},
    routes::jobs::{ApiError, JobIdPath},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::State};
use metis_common::api::v1::{
    job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse},
    task_status::TaskStatusLog as ApiTaskStatusLog,
};
use tracing::{error, info};

pub async fn set_job_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    JobIdPath(job_id): JobIdPath,
    Json(status): Json<JobStatusUpdate>,
) -> Result<Json<SetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, status = ?status, "set_job_status invoked");

    let response = with_actor(Some(actor.name()), async {
        state
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
                SetJobStatusError::InvalidStatusTransition { job_id } => {
                    info!(job_id = %job_id, "invalid status transition for task");
                    ApiError::conflict(format!("Invalid status transition for job '{job_id}'"))
                }
                SetJobStatusError::Store { source, job_id } => {
                    error!(error = %source, job_id = %job_id, "failed to update task status");
                    ApiError::internal(anyhow!("Failed to update task status: {source}"))
                }
                SetJobStatusError::PolicyViolation(violation) => {
                    ApiError::bad_request(violation.message)
                }
            })
    })
    .await?;

    info!(job_id = %response.job_id, "job status stored successfully");
    Ok(Json(response))
}

pub async fn get_job_status(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<GetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, "get_job_status invoked");

    state.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to load task for job status");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    let status_log = state.get_status_log(&job_id).await.map_err(|err| {
        error!(
            error = %err,
            job_id = %job_id,
            "failed to load status log for job status"
        );
        ApiError::internal(anyhow!("Failed to load status log: {err}"))
    })?;

    let status_log: ApiTaskStatusLog = status_log.into();
    info!(job_id = %job_id, "get_job_status completed");
    Ok(Json(GetJobStatusResponse::new(job_id, status_log)))
}
