use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::job_outputs::{JobOutputPayload, JobOutputResponse};
use tracing::{error, info};

pub async fn set_job_output(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
    Json(payload): Json<JobOutputPayload>,
) -> Result<Json<JobOutputResponse>, ApiError> {
    info!(job_id = %job_id, "set_job_output invoked");

    // Mark the task as complete with the CodexOutput value
    {
        let mut store = state.store.write().await;

        // Verify task exists
        store.get_task(&job_id).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task for output");
            ApiError::not_found(format!("Job '{job_id}' not found in store"))
        })?;

        // Mark task as complete with the JobOutputPayload
        store
            .mark_task_complete(&job_id, Ok(payload.clone()), Utc::now())
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to mark task complete with output");
                ApiError::internal(anyhow::anyhow!("Failed to mark task complete: {err}"))
            })?;
    }

    info!(job_id = %job_id, "job output stored successfully");
    Ok(Json(JobOutputResponse {
        job_id,
        output: payload,
    }))
}

pub async fn get_job_output(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<JobOutputResponse>, ApiError> {
    info!(job_id = %job_id, "get_job_output invoked");

    let store = state.store.read().await;

    // Verify task exists
    store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    // Get the result from the task
    let output = match store.get_result(&job_id) {
        Some(Ok(output)) => output,
        Some(Err(e)) => {
            error!(error = ?e, job_id = %job_id, "task completed with error");
            return Err(ApiError::internal(anyhow::anyhow!(
                "Task completed with error: {e:?}"
            )));
        }
        None => {
            error!(job_id = %job_id, "job output not available");
            return Err(ApiError::bad_request(format!(
                "Job '{job_id}' has not completed yet."
            )));
        }
    };

    info!(job_id = %job_id, "job output found");
    Ok(Json(JobOutputResponse { job_id, output }))
}
