use crate::{AppState, routes::jobs::ApiError, store::Task};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::job_outputs::{JobOutputPayload, JobOutputResponse, JobOutputType};
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

    // Get the current task, update it with the result, and store it back
    let output_type = {
        let mut store = state.store.write().await;
        let job_id_string = job_id.to_string();
        let current_task = store.get_task(&job_id_string).await.map_err(|err| {
            error!(error = %err, job_id = %job_id, "failed to get task for output");
            ApiError::not_found(format!("Job '{job_id}' not found in store"))
        })?;

        let (output_type, updated_task) = match current_task {
            Task::Spawn {
                prompt,
                context,
                output_type,
                ..
            } => {
                if !matches!(output_type, JobOutputType::Patch) {
                    error!(
                        job_id = %job_id,
                        ?output_type,
                        "set_job_output called for unsupported output type"
                    );
                    return Err(ApiError::bad_request(format!(
                        "Output type '{output_type:?}' is not supported"
                    )));
                }
                (
                    output_type,
                    Task::Spawn {
                        prompt,
                        context,
                        output_type,
                        result: Some(payload.clone()),
                    },
                )
            }
            Task::Ask => {
                error!(job_id = %job_id, "attempted to set output on Ask task");
                return Err(ApiError::bad_request("Cannot set output on Ask task"));
            }
        };

        store
            .update_task(&job_id_string, updated_task)
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to update task with output");
                ApiError::internal(anyhow::anyhow!("Failed to update task: {err}"))
            })?;
        output_type
    };

    info!(job_id = %job_id, "job output stored successfully");
    Ok(Json(JobOutputResponse {
        job_id: job_id.to_string(),
        output_type,
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
    let task = store.get_task(&job_id_string).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    if let Task::Spawn {
        output_type,
        result: Some(output),
        ..
    } = task
    {
        if !matches!(output_type, JobOutputType::Patch) {
            error!(
                job_id = %job_id,
                ?output_type,
                "get_job_output called for unsupported output type"
            );
            return Err(ApiError::bad_request(format!(
                "Output type '{output_type:?}' is not supported"
            )));
        }
        info!(job_id = %job_id, "job output found");
        return Ok(Json(JobOutputResponse {
            job_id: job_id.to_string(),
            output_type,
            output: output.clone(),
        }));
    }

    error!(job_id = %job_id, "job output not available");
    Err(ApiError::bad_request(format!(
        "Job '{job_id}' has not completed yet."
    )))
}
