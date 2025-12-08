use crate::{
    AppState,
    store::{Store, StoreError, Task},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use metis_common::{
    job_outputs::JobOutputType,
    jobs::{CreateJobRequest, CreateJobResponse, JobSummary, ListJobsResponse},
};
use serde_json::json;
use tracing::{error, info};

pub mod context;
pub mod kill;
pub mod logs;
pub mod output;

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

    let parent_ids: Vec<String> = payload
        .parent_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .collect();
    if parent_ids.iter().any(|id| id.is_empty()) {
        error!("create_job received an empty parent_id");
        return Err(ApiError::bad_request("parent_ids must not be empty"));
    }

    // Generate a unique ID for the job
    let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

    // Store the task with context and prompt (status will be Pending)
    {
        let mut store = state.store.write().await;
        let task = Task::Spawn {
            prompt: prompt.clone(),
            context: payload.context.clone(),
            output_type: payload.output_type,
            result: None,
        };
        store
            .add_task_with_id(job_id.clone(), task, parent_ids.clone(), Utc::now())
            .await
            .map_err(|err| match err {
                StoreError::InvalidDependency(msg) => {
                    error!(
                        error = %msg,
                        job_id = %job_id,
                        "failed to store task due to invalid parent dependency"
                    );
                    ApiError::bad_request(msg)
                }
                err => {
                    error!(error = %err, job_id = %job_id, "failed to store task");
                    ApiError::internal(anyhow::anyhow!("Failed to store task: {err}"))
                }
            })?;
    }

    info!(
        job_id = %job_id,
        parent_count = parent_ids.len(),
        "task stored, will be started by background thread"
    );

    Ok(Json(CreateJobResponse { job_id }))
}

pub async fn list_jobs(State(state): State<AppState>) -> Result<Json<ListJobsResponse>, ApiError> {
    info!("list_jobs invoked");
    let config = state.config;
    let namespace = config.metis.namespace.clone();

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    // Get all tasks with all statuses
    let task_ids = store.list_tasks().await.map_err(|err| {
        error!(error = %err, "failed to list tasks");
        ApiError::internal(anyhow::anyhow!("Failed to list tasks: {err}"))
    })?;

    // Collect all summaries with their reference times for sorting
    let mut summaries_with_times: Vec<(JobSummary, Option<DateTime<Utc>>)> = Vec::new();
    for task_id in task_ids {
        match job_summary_with_time(&task_id, store).await {
            Ok(summary) => summaries_with_times.push(summary),
            Err(err) => {
                error!(
                    job_id = %task_id,
                    error = %err,
                    "failed to build summary while listing jobs"
                );
                continue;
            }
        }
    }

    // Sort by reference time, most recent first
    summaries_with_times.sort_by(|a, b| {
        let time_a = a.1;
        let time_b = b.1;
        time_b.cmp(&time_a)
    });

    let summaries: Vec<JobSummary> = summaries_with_times
        .into_iter()
        .map(|(summary, _)| summary)
        .collect();

    info!(
        namespace = %namespace,
        job_count = summaries.len(),
        "list_jobs completed successfully"
    );

    Ok(Json(ListJobsResponse { jobs: summaries }))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobSummary>, ApiError> {
    info!(job_id = %job_id, "get_job invoked");
    let job_id = job_id.trim();
    if job_id.is_empty() {
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    let (summary, _) = job_summary_with_time(job_id, store)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) => {
                error!(job_id = %job_id, "job not found");
                ApiError::not_found(format!("job '{job_id}' not found"))
            }
            err => {
                error!(job_id = %job_id, error = %err, "failed to load job summary");
                ApiError::internal(anyhow::anyhow!("Failed to load job '{job_id}': {err}"))
            }
        })?;

    info!(job_id = %summary.id, "get_job completed successfully");
    Ok(Json(summary))
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

async fn job_summary_with_time(
    job_id: &str,
    store: &dyn Store,
) -> Result<(JobSummary, Option<DateTime<Utc>>), StoreError> {
    let job_id = job_id.to_string();
    let status_log = store.get_status_log(&job_id).await?;
    let notes = job_notes_from_store(&job_id, store).await;
    let output_type = match store.get_task(&job_id).await? {
        Task::Spawn { output_type, .. } => output_type,
        Task::Ask => JobOutputType::Patch,
    };

    let reference_time = status_log.start_time.or(Some(status_log.creation_time));

    Ok((
        JobSummary {
            id: job_id,
            notes,
            output_type,
            status_log,
        },
        reference_time,
    ))
}

async fn job_notes_from_store(
    job_id: &str,
    store: &dyn Store,
) -> Option<String> {
    let job_id_string = job_id.to_string();
    if let Ok(Task::Spawn {
        result: Some(output),
        ..
    }) = store.get_task(&job_id_string).await
    {
        return sanitize_note(&output.last_message);
    }

    None
}

fn sanitize_note(note: &str) -> Option<String> {
    let collapsed = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}
