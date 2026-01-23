use crate::{
    app::{AppState, TaskExt},
    domain::{issues::JobSettings, jobs::WorkerContext},
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

    let (mut task, issue_job_settings) = {
        let store = state.store.read().await;
        let task = store.get_task(&job_id).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task");
            ApiError::not_found(format!("Job '{job_id}' not found"))
        })?;

        let issue_job_settings = match task.spawned_from.clone() {
            Some(issue_id) => match store.get_issue(&issue_id).await {
                Ok(issue) => Some(issue.job_settings),
                Err(StoreError::IssueNotFound(_)) => {
                    return Err(ApiError::not_found(format!("issue '{issue_id}' not found")));
                }
                Err(err) => {
                    error!(error = %err, issue_id = %issue_id, "failed to load issue");
                    return Err(ApiError::internal(err));
                }
            },
            None => None,
        };

        (task, issue_job_settings)
    };

    if let Some(settings) = issue_job_settings {
        let merged = JobSettings::merge(task.job_settings.clone(), settings);
        task.job_settings = merged;
    }

    let resolved = task
        .resolve_context(state.service_state.as_ref())
        .await
        .map_err(ApiError::from)?;
    let env_vars = task.resolve_env_vars(&resolved);

    let context: v1::jobs::WorkerContext =
        WorkerContext::new(resolved.bundle, task.prompt, env_vars).into();
    info!(job_id = %job_id, "get_job_context completed");
    Ok(Json(context))
}
