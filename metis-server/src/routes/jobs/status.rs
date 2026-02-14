use crate::domain::actors::Actor;
use crate::{
    app::{AppState, SetJobStatusError},
    routes::jobs::{ApiError, JobIdPath},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::State};
use metis_common::api::v1::job_status::{JobStatusUpdate, SetJobStatusResponse};
use tracing::{error, info};

pub async fn set_job_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    JobIdPath(job_id): JobIdPath,
    Json(status): Json<JobStatusUpdate>,
) -> Result<Json<SetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, status = ?status, "set_job_status invoked");

    let response = state
        .set_job_status(job_id, status, Some(actor.name()))
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
        })?;

    info!(job_id = %response.job_id, "job status stored successfully");
    Ok(Json(response))
}
