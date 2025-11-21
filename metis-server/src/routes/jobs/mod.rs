use crate::{
    AppState,
    job_store::JobStoreError,
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Duration as ChronoDuration, Utc};
use metis_common::{
    job_outputs::JobOutputPayload,
    jobs::{CreateJobRequest, CreateJobResponse, JobSummary, ListJobsResponse},
};
use serde_json::json;
use std::collections::HashMap;
use tracing::{error, info};

pub mod logs;
pub mod output;
pub mod context;

pub async fn create_job(
    State(state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    info!("create_job invoked");
    let prompt = payload.prompt.trim().to_string();
    if prompt.is_empty() {
        error!("create_job received an empty prompt");
        return Err(ApiError::bad_request("prompt is required"));
    }

    let result = state.job_store.create_job(&prompt).await
        .map_err(|err| match err {
            JobStoreError::AlreadyExists(msg) => {
                error!(error = %msg, "job already exists");
                ApiError::conflict(msg)
            }
            JobStoreError::Kubernetes(kube_err) => {
                error!(error = ?kube_err, "failed to create job in Kubernetes");
                ApiError::internal(kube_err)
            }
            err => {
                error!(error = %err, "failed to create job");
                ApiError::internal(err)
            }
        })?;

    // Store the job context for later retrieval
    {
        let mut ctx_store = state.job_contexts.write().await;
        ctx_store.insert(result.job_id.clone(), payload.context.clone());
    }

    Ok(Json(CreateJobResponse {
        job_id: result.job_id,
        job_name: result.job_name,
        namespace: result.namespace,
    }))
}

pub async fn list_jobs(State(state): State<AppState>) -> Result<Json<ListJobsResponse>, ApiError> {
    info!("list_jobs invoked");
    let config = state.config;
    let namespace = config.metis.namespace.clone();
    
    let metis_jobs = state.job_store.list_jobs().await
        .map_err(|err| {
            error!(error = ?err, namespace = %namespace, "failed to list jobs");
            match err {
                JobStoreError::Kubernetes(kube_err) => {
                    error!(error = ?kube_err, "Kubernetes error in list_jobs");
                    ApiError::internal(kube_err)
                }
                err => {
                    error!(error = %err, "error listing jobs");
                    ApiError::internal(err)
                }
            }
        })?;

    let now = Utc::now();
    let job_outputs = {
        let store = state.job_outputs.read().await;
        store.clone()
    };

    let summaries: Vec<JobSummary> = metis_jobs
        .into_iter()
        .map(|job| {
            let runtime = job_runtime(&job, now).map(format_duration);
            let notes = job_notes(&job.id, &job.status, &job.failure_message, &job_outputs);

            JobSummary {
                id: job.id,
                status: job.status,
                runtime,
                notes,
            }
        })
        .collect();

    info!(
        namespace = %namespace,
        job_count = summaries.len(),
        "list_jobs completed successfully"
    );

    Ok(Json(ListJobsResponse {
        namespace,
        jobs: summaries,
    }))
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn internal(error: impl Into<anyhow::Error>) -> Self {
        let err = error.into();
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}


fn job_runtime(job: &crate::job_store::MetisJob, now: chrono::DateTime<Utc>) -> Option<ChronoDuration> {
    let start = job.start_time.or(job.creation_time)?;
    let end = job.completion_time.unwrap_or(now);

    if end < start {
        return Some(ChronoDuration::zero());
    }

    Some(end - start)
}

fn format_duration(duration: ChronoDuration) -> String {
    let total_seconds = duration.num_seconds();
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn job_notes(
    job_id: &str,
    status: &str,
    failure_message: &Option<String>,
    outputs: &HashMap<String, JobOutputPayload>,
) -> Option<String> {
    let note = match status {
        "failed" => {
            failure_message.clone().or_else(|| outputs.get(job_id).map(|o| o.last_message.clone()))
        }
        "complete" => outputs.get(job_id).map(|o| o.last_message.clone()),
        "running" => outputs.get(job_id).map(|o| o.last_message.clone()),
        _ => None,
    }?;

    sanitize_note(&note)
}

fn sanitize_note(note: &str) -> Option<String> {
    let collapsed = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

