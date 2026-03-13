use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{AppState, SetSessionStatusError},
    routes::sessions::{ApiError, SessionIdPath},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::State};
use metis_common::api::v1::session_status::{SessionStatusUpdate, SetSessionStatusResponse};
use tracing::{error, info};

pub async fn set_session_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    Json(status): Json<SessionStatusUpdate>,
) -> Result<Json<SetSessionStatusResponse>, ApiError> {
    info!(session_id = %session_id, status = ?status, "set_session_status invoked");

    let response = state
        .set_session_status(session_id, status, ActorRef::from(&actor))
        .await
        .map_err(|err| match err {
            SetSessionStatusError::NotFound { source, session_id } => {
                error!(
                    error = %source,
                    session_id = %session_id,
                    "failed to get task for status update"
                );
                ApiError::not_found(format!("Session '{session_id}' not found in store"))
            }
            SetSessionStatusError::InvalidStatusTransition { session_id } => {
                info!(session_id = %session_id, "invalid status transition for task");
                ApiError::conflict(format!(
                    "Invalid status transition for session '{session_id}'"
                ))
            }
            SetSessionStatusError::Store { source, session_id } => {
                error!(error = %source, session_id = %session_id, "failed to update task status");
                ApiError::internal(anyhow!("Failed to update task status: {source}"))
            }
            SetSessionStatusError::PolicyViolation(violation) => {
                ApiError::bad_request(violation.message)
            }
        })?;

    info!(session_id = %response.session_id, "session status stored successfully");
    Ok(Json(response))
}
