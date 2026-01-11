use crate::{
    AppState,
    routes::{ApiError, artifacts::ArtifactIdPath},
    store::{Event, StoreError, TaskError},
};
use anyhow::anyhow;
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::artifact_status::{
    ArtifactStatusUpdate, GetArtifactStatusResponse, SetArtifactStatusResponse,
};
use tracing::{error, info};

pub async fn set_artifact_status(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
    Json(status): Json<ArtifactStatusUpdate>,
) -> Result<Json<SetArtifactStatusResponse>, ApiError> {
    info!(
        artifact_id = %artifact_id,
        status = ?status,
        "set_artifact_status invoked"
    );

    {
        let mut store = state.store.write().await;

        if let Err(err) = store.get_artifact(&artifact_id).await {
            error!(
                error = %err,
                artifact_id = %artifact_id,
                "failed to get artifact for status update"
            );
            return Err(ApiError::not_found(format!(
                "Artifact '{artifact_id}' not found in store"
            )));
        };

        let event = match &status {
            ArtifactStatusUpdate::Complete => Event::Completed { at: Utc::now() },
            ArtifactStatusUpdate::Failed { reason } => Event::Failed {
                at: Utc::now(),
                error: TaskError::JobEngineError {
                    reason: reason.clone(),
                },
            },
        };

        store
            .append_status_event(&artifact_id, event)
            .await
            .map_err(|err| match err {
                StoreError::TaskNotFound(_) | StoreError::ArtifactNotFound(_) => {
                    error!(
                        error = %err,
                        artifact_id = %artifact_id,
                        "artifact missing while updating status"
                    );
                    ApiError::not_found(format!("Artifact '{artifact_id}' not found in store"))
                }
                StoreError::InvalidStatusTransition => {
                    error!(
                        error = %err,
                        artifact_id = %artifact_id,
                        "invalid status transition for artifact"
                    );
                    ApiError::bad_request("artifact status cannot transition from current state")
                }
                other => {
                    error!(
                        error = %other,
                        artifact_id = %artifact_id,
                        "failed to update task status"
                    );
                    ApiError::internal(anyhow!("Failed to update task status: {other}"))
                }
            })?;
    }

    info!(
        artifact_id = %artifact_id,
        "artifact status stored successfully"
    );
    Ok(Json(SetArtifactStatusResponse {
        artifact_id,
        status: status.as_status(),
    }))
}

pub async fn get_artifact_status(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
) -> Result<Json<GetArtifactStatusResponse>, ApiError> {
    info!(artifact_id = %artifact_id, "get_artifact_status invoked");

    let store = state.store.read().await;
    if let Err(err) = store.get_artifact(&artifact_id).await {
        error!(
            error = %err,
            artifact_id = %artifact_id,
            "failed to load artifact for status"
        );
        return Err(ApiError::not_found(format!(
            "Artifact '{artifact_id}' not found"
        )));
    };

    let status_log = store
        .get_status_log(&artifact_id)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) | StoreError::ArtifactNotFound(_) => {
                ApiError::not_found(format!("Artifact '{artifact_id}' not found"))
            }
            other => {
                error!(
                    error = %other,
                    artifact_id = %artifact_id,
                    "failed to load status log for artifact"
                );
                ApiError::internal(anyhow!("Failed to load status log: {other}"))
            }
        })?;

    Ok(Json(GetArtifactStatusResponse {
        artifact_id,
        status_log,
    }))
}
