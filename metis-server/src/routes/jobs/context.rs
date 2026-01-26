use crate::{
    app::AppState,
    domain::jobs::WorkerContext,
    routes::jobs::{ApiError, JobIdPath},
    store::StoreError,
};
use axum::{Json, extract::State};
use metis_common::api::v1;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<v1::jobs::WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let task = {
        let task = state.get_task(&job_id).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task");
            ApiError::not_found(format!("Job '{job_id}' not found"))
        })?;

        if let Some(issue_id) = task.spawned_from.clone() {
            match state.get_issue(&issue_id).await {
                Ok(_) => {}
                Err(StoreError::IssueNotFound(_)) => {
                    return Err(ApiError::not_found(format!("issue '{issue_id}' not found")));
                }
                Err(err) => {
                    error!(error = %err, issue_id = %issue_id, "failed to load issue");
                    return Err(ApiError::internal(err));
                }
            }
        }

        task
    };

    let resolved = state.resolve_task(&task).await.map_err(ApiError::from)?;

    let context: v1::jobs::WorkerContext =
        WorkerContext::new(resolved.context.bundle, task.prompt, resolved.env_vars).into();
    info!(job_id = %job_id, "get_job_context completed");
    Ok(Json(context))
}
