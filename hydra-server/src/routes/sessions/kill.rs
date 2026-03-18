use crate::{app::AppState, job_engine::JobEngineError};
use axum::{Json, extract::State};
use hydra_common::api::v1;
use tracing::{error, info};

use super::{ApiError, SessionIdPath};

pub async fn kill_session(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::KillSessionResponse>, ApiError> {
    info!(session_id = %session_id, "kill_session invoked");
    state
        .job_engine
        .kill_job(&session_id)
        .await
        .map_err(|err| match err {
            JobEngineError::NotFound(hydra_id) => {
                let message = format!("Session '{hydra_id}' not found");
                error!(session_id = %session_id, error = %message, "session not found");
                ApiError::not_found(message)
            }
            JobEngineError::MultipleFound(hydra_id) => {
                let message = format!("Multiple sessions found for hydra-id '{hydra_id}'");
                error!(session_id = %session_id, error = %message, "multiple sessions found");
                ApiError::conflict(message)
            }
            #[cfg(feature = "kubernetes")]
            JobEngineError::Kubernetes(kube_err) => {
                error!(session_id = %session_id, error = ?kube_err, "kubernetes error while killing session");
                ApiError::internal(kube_err)
            }
            other => {
                error!(session_id = %session_id, error = %other, "failed to kill session");
                ApiError::internal(other)
            }
        })?;

    info!(session_id = %session_id, "kill_session completed successfully");

    Ok(Json(v1::sessions::KillSessionResponse::new(
        session_id,
        "killed".to_string(),
    )))
}
