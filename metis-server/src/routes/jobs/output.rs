use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath},
    store::Task,
};
use anyhow::anyhow;
use axum::{Json, extract::State};
use chrono::Utc;
use metis_common::{MetisId, artifacts::Artifact, job_outputs::SetJobOutputResponse, jobs::Bundle};
use serde::Serialize;
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

#[derive(Debug, Serialize)]
pub(crate) struct JobOutput {
    last_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<String>,
    bundle: Bundle,
}

#[derive(Debug, Serialize)]
pub(crate) struct JobOutputResponse {
    job_id: MetisId,
    output: JobOutput,
}

pub async fn get_job_output(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<JobOutputResponse>, ApiError> {
    info!(job_id = %job_id, "get_job_output invoked");

    let store = state.store.read().await;
    let task = store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to load task for job output");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    let status_log = store.get_status_log(&job_id).await.map_err(|err| {
        error!(
            error = %err,
            job_id = %job_id,
            "failed to load status log for job output"
        );
        ApiError::internal(anyhow!("Failed to load status log: {err}"))
    })?;

    let artifact_ids = status_log
        .emitted_artifacts()
        .ok_or_else(|| ApiError::not_found(format!("Output not available for job '{job_id}'")))?;

    let bundle = match task {
        Task::Spawn { context, .. } => context,
    };

    let mut last_message = None;
    let mut patch = None;

    for artifact_id in artifact_ids {
        match store.get_artifact(&artifact_id).await {
            Ok(Artifact::Patch { diff, description }) => {
                last_message = Some(description);
                patch = Some(diff);
                break;
            }
            Ok(Artifact::Issue { description }) => {
                if last_message.is_none() {
                    last_message = Some(description);
                }
            }
            Err(err) => {
                error!(
                    error = %err,
                    job_id = %job_id,
                    artifact_id = %artifact_id,
                    "failed to fetch artifact for job output"
                );
            }
        }
    }

    let last_message = last_message
        .ok_or_else(|| ApiError::not_found(format!("Artifacts for job '{job_id}' not found")))?;

    Ok(Json(JobOutputResponse {
        job_id,
        output: JobOutput {
            last_message,
            patch,
            bundle,
        },
    }))
}
