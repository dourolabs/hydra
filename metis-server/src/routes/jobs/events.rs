use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::MetisId;
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info};

#[derive(Debug, Deserialize)]
pub struct EmitEventRequest {
    pub artifact_ids: Vec<MetisId>,
}

pub async fn emit_artifacts(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
    Json(payload): Json<EmitEventRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    info!(job_id = %job_id, count = payload.artifact_ids.len(), "emit_artifacts invoked");

    if payload.artifact_ids.is_empty() {
        return Err(ApiError::bad_request(
            "artifact_ids must not be empty when emitting artifacts",
        ));
    }

    let mut store = state.store.write().await;
    store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to fetch task for emission");
        ApiError::not_found(format!("Job '{job_id}' not found in store"))
    })?;

    for artifact_id in &payload.artifact_ids {
        store.get_artifact(artifact_id).await.map_err(|err| {
            error!(artifact_id = %artifact_id, error = %err, job_id = %job_id, "artifact missing when emitting");
            ApiError::bad_request(format!("Artifact '{artifact_id}' not found"))
        })?;
    }

    store
        .emit_task_artifacts(&job_id, payload.artifact_ids.clone(), Utc::now())
        .await
        .map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to record emitted artifacts");
            ApiError::internal(anyhow::anyhow!("Failed to record emitted artifacts: {err}"))
        })?;

    Ok(Json(json!({
        "job_id": job_id,
        "artifact_ids": payload.artifact_ids,
    })))
}
