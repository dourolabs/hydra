use crate::{AppState, routes::jobs::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
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
    if payload.last_message.trim().is_empty() {
        error!(
            job_id = %job_id,
            "set_job_output received an empty last_message"
        );
        return Err(ApiError::bad_request("last_message must not be empty"));
    }
    if payload.patch.trim().is_empty() {
        error!(job_id = %job_id, "set_job_output received an empty patch");
        return Err(ApiError::bad_request("patch must not be empty"));
    }

    let mut store = state.job_outputs.write().await;
    store.insert(job_id.to_string(), payload.clone());
    drop(store);

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

    let store = state.job_outputs.read().await;
    if let Some(output) = store.get(job_id) {
        info!(job_id = %job_id, "job output found");
        return Ok(Json(JobOutputResponse {
            job_id: job_id.to_string(),
            output: output.clone(),
        }));
    }

    error!(job_id = %job_id, "job output not available");
    Err(ApiError::bad_request(format!(
        "Job '{job_id}' has not completed yet."
    )))
}
