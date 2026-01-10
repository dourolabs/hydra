use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use anyhow::anyhow;
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::artifacts::Artifact;
use metis_common::job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse};
use tracing::{error, info};

pub async fn set_job_status(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
    Json(status): Json<JobStatusUpdate>,
) -> Result<Json<SetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, status = ?status, "set_job_status invoked");

    {
        let mut store = state.store.write().await;

        match store.get_artifact(&job_id).await {
            Ok(Artifact::Session { .. }) => {}
            Ok(other) => {
                error!(job_id = %job_id, artifact = ?other, "artifact for job status update was not a session");
                return Err(ApiError::not_found(format!(
                    "Job '{job_id}' not found in store"
                )));
            }
            Err(err) => {
                error!(error = %err, job_id = %job_id, "failed to get artifact for status update");
                return Err(ApiError::not_found(format!(
                    "Job '{job_id}' not found in store"
                )));
            }
        };

        let result = status.to_result();

        store
            .mark_task_complete(&job_id, result, Utc::now())
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to update task status");
                ApiError::internal(anyhow!("Failed to update task status: {err}"))
            })?;
    }

    info!(job_id = %job_id, "job status stored successfully");
    Ok(Json(SetJobStatusResponse {
        job_id,
        status: status.as_status(),
    }))
}

pub async fn get_job_status(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<GetJobStatusResponse>, ApiError> {
    info!(job_id = %job_id, "get_job_status invoked");

    let store = state.store.read().await;
    match store.get_artifact(&job_id).await {
        Ok(Artifact::Session { .. }) => {}
        Ok(other) => {
            error!(job_id = %job_id, artifact = ?other, "artifact for job status was not a session");
            return Err(ApiError::not_found(format!("Job '{job_id}' not found")));
        }
        Err(err) => {
            error!(error = %err, job_id = %job_id, "failed to load artifact for job status");
            return Err(ApiError::not_found(format!("Job '{job_id}' not found")));
        }
    };

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
