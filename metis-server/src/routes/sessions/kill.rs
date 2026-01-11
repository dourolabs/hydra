use crate::{AppState, job_engine::JobEngineError, routes::ApiError};
use axum::{Json, extract::State};
use metis_common::sessions::KillSessionResponse;
use tracing::{error, info};

use super::SessionIdPath;

pub async fn kill_session(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<KillSessionResponse>, ApiError> {
    state
        .job_engine
        .kill_job(&session_id)
        .await
        .map_err(|err| match err {
            JobEngineError::NotFound(metis_id) => {
                let message = format!("Session '{metis_id}' not found");
                error!(session_id = %session_id, error = %message, "session not found");
                ApiError::not_found(message)
            }
            JobEngineError::MultipleFound(metis_id) => {
                let message = format!("Multiple sessions found for metis-id '{metis_id}'");
                error!(
                    session_id = %session_id,
                    error = %message,
                    "multiple sessions found"
                );
                ApiError::conflict(message)
            }
            JobEngineError::Kubernetes(kube_err) => {
                error!(
                    session_id = %session_id,
                    error = ?kube_err,
                    "kubernetes error while killing session"
                );
                ApiError::internal(kube_err)
            }
            other => {
                error!(session_id = %session_id, error = %other, "failed to kill session");
                ApiError::internal(other)
            }
        })?;

    info!(session_id = %session_id, "session killed successfully");

    Ok(Json(KillSessionResponse {
        session_id,
        status: "killed".to_string(),
    }))
}
