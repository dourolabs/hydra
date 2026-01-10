use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
};
use axum::{Json, extract::State};
use metis_common::artifacts::Artifact;
use metis_common::jobs::WorkerContext;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let store = state.store.read().await;
    let artifact = store.get_artifact(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get artifact");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    match artifact {
        Artifact::Session {
            program,
            params,
            context,
            env_vars,
            ..
        } => Ok(Json(WorkerContext {
            request_context: context.clone(),
            program: program.clone(),
            params: params.clone(),
            variables: env_vars.clone(),
        })),
        other => {
            error!(job_id = %job_id, artifact = ?other, "artifact for job context was not a session");
            Err(ApiError::not_found(format!("Job '{job_id}' not found")))
        }
    }
}
