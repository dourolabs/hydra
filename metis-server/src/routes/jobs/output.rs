use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::job_outputs::SetJobOutputResponse;
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
