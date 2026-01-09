use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath, payload_from_artifact},
};
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::MetisId;
use metis_common::job_outputs::{JobOutputPayload, JobOutputResponse, SetJobOutputResponse};
use tracing::warn;
use tracing::{error, info};

pub async fn set_job_output(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<SetJobOutputResponse>, ApiError> {
    info!(job_id = %job_id, "set_job_output invoked");

    {
        let mut store = state.store.write().await;

        store.get_task(&job_id).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task for output");
            ApiError::not_found(format!("Job '{job_id}' not found in store"))
        })?;

        store
            .mark_task_complete(&job_id, Ok(()), Utc::now())
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to mark task complete with output");
                ApiError::internal(anyhow::anyhow!("Failed to mark task complete: {err}"))
            })?;
    }

    info!(job_id = %job_id, "job output stored successfully");
    Ok(Json(SetJobOutputResponse { job_id }))
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
        Some(Ok(())) => resolve_latest_output(&job_id, &**store).await?,
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

async fn resolve_latest_output(
    job_id: &MetisId,
    store: &dyn crate::store::Store,
) -> Result<JobOutputPayload, ApiError> {
    let artifact_ids = store
        .latest_emitted_artifact_ids(job_id)
        .await
        .map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to fetch emitted artifacts");
            ApiError::internal(anyhow::anyhow!("Failed to fetch emitted artifacts: {err}"))
        })?
        .ok_or_else(|| {
            warn!(job_id = %job_id, "job has not emitted any artifacts yet");
            ApiError::bad_request(format!("Job '{job_id}' has not emitted any artifacts yet."))
        })?;

    for artifact_id in artifact_ids {
        let artifact = store.get_artifact(&artifact_id).await.map_err(|err| {
            error!(error = %err, artifact_id = %artifact_id, job_id = %job_id, "failed to load artifact");
            ApiError::internal(anyhow::anyhow!(
                "Failed to load artifact {artifact_id}: {err}"
            ))
        })?;

        if let Some(output) = payload_from_artifact(&artifact) {
            return Ok(output);
        }
    }

    Err(ApiError::internal(anyhow::anyhow!(
        "No usable patch artifacts found for job {job_id}"
    )))
}
