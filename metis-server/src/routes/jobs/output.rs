use crate::{AppState, routes::jobs::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use metis_common::job_outputs::{JobOutputPayload, JobOutputResponse};
use tracing::{error, info};

pub async fn set_job_output(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Json(payload): Json<JobOutputPayload>,
) -> Result<Json<JobOutputResponse>, ApiError> {
    let job_id = job_id.trim();
    info!(job_id = %job_id, "set_job_output invoked");
    if job_id.is_empty() {
        error!("set_job_output received an empty job_id");
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    // Mark the task as complete with the CodexOutput value
    {
        let mut store = state.store.write().await;
        let job_id_string = job_id.to_string();
        
        // Verify task exists
        store.get_task(&job_id_string).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task for output");
            ApiError::not_found(format!("Job '{job_id}' not found in store"))
        })?;

        // Mark task as complete with the JobOutputPayload
        store
            .mark_task_complete(&job_id_string, Ok(payload.clone()), Utc::now())
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to mark task complete with output");
                ApiError::internal(anyhow::anyhow!("Failed to mark task complete: {err}"))
            })?;
    }

    info!(job_id = %job_id, "job output stored successfully");
    Ok(Json(JobOutputResponse {
        job_id: job_id.to_string(),
        output: payload,
    }))
}

pub async fn get_job_output(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobOutputResponse>, ApiError> {
    let job_id = job_id.trim();
    info!(job_id = %job_id, "get_job_output invoked");
    if job_id.is_empty() {
        error!("get_job_output received an empty job_id");
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    let store = state.store.read().await;
    let job_id_string = job_id.to_string();
    
    // Verify task exists
    store.get_task(&job_id_string).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    // Get the result from the task
    let output = match store.get_result(&job_id_string) {
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
    Ok(Json(JobOutputResponse {
        job_id: job_id.to_string(),
        output,
    }))
}
