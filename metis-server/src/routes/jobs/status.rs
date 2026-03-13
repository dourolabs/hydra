use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{AppState, SetSessionStatusError},
    routes::jobs::{ApiError, JobIdPath},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::State};
use metis_common::api::v1::session_status::{SessionStatusUpdate, SetSessionStatusResponse};
use tracing::{error, info};

pub async fn set_job_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    JobIdPath(job_id): JobIdPath,
    Json(status): Json<SessionStatusUpdate>,
) -> Result<Json<SetSessionStatusResponse>, ApiError> {
    info!(job_id = %job_id, status = ?status, "set_job_status invoked");

    let response = state
        .set_session_status(job_id, status, ActorRef::from(&actor))
        .await
        .map_err(|err| match err {
            SetSessionStatusError::NotFound { source, session_id } => {
                error!(
                    error = %source,
                    job_id = %session_id,
                    "failed to get task for status update"
                );
                ApiError::not_found(format!("Job '{session_id}' not found in store"))
            }
            SetSessionStatusError::InvalidStatusTransition { session_id } => {
                info!(job_id = %session_id, "invalid status transition for task");
                ApiError::conflict(format!("Invalid status transition for job '{session_id}'"))
            }
            SetSessionStatusError::Store { source, session_id } => {
                error!(error = %source, job_id = %session_id, "failed to update task status");
                ApiError::internal(anyhow!("Failed to update task status: {source}"))
            }
            SetSessionStatusError::PolicyViolation(violation) => {
                ApiError::bad_request(violation.message)
            }
        })?;

    info!(job_id = %response.session_id, "job status stored successfully");
    Ok(Json(response))
}
