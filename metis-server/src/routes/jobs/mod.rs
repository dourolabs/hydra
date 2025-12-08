use crate::{
    AppState,
    store::{Status as StoreStatus, Store, StoreError, Task},
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
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

    let now = Utc::now();

    // Collect all summaries with their reference times for sorting
    let mut summaries_with_times: Vec<(JobSummary, Option<DateTime<Utc>>)> = Vec::new();
    for task_id in task_ids {
        let status = match store.get_status(&task_id).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let status_log = match store.get_status_log(&task_id).await {
            Ok(log) => log,
            Err(_) => continue,
        };

        let job_status_str = match status {
            StoreStatus::Running => "running",
            StoreStatus::Complete => "complete",
            StoreStatus::Failed => "failed",
            StoreStatus::Pending => "pending",
            StoreStatus::Blocked => "blocked",
        };

        let runtime = task_runtime(&status_log, now).map(format_duration);
        let notes =
            job_notes_from_store(&task_id, &status, &status_log.failure_reason, store).await;
        let output_type = match store.get_task(&task_id).await {
            Ok(Task::Spawn { output_type, .. }) => output_type,
            Ok(Task::Ask) => JobOutputType::Patch,
            Err(_) => continue,
        };

        let reference_time = status_log.start_time.or(Some(status_log.creation_time));
        summaries_with_times.push((
            JobSummary {
                id: task_id,
                status: job_status_str.to_string(),
                runtime,
                notes,
                output_type,
            },
            reference_time,
        ));
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

fn task_runtime(
    status_log: &crate::store::TaskStatusLog,
    now: DateTime<Utc>,
) -> Option<ChronoDuration> {
    let start = status_log.start_time.or(Some(status_log.creation_time))?;
    let end = status_log.end_time.unwrap_or(now);

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

async fn job_notes_from_store(
    job_id: &str,
    status: &StoreStatus,
    failure_reason: &Option<String>,
    store: &dyn Store,
) -> Option<String> {
    let note = match status {
        StoreStatus::Failed => failure_reason.clone(),
        StoreStatus::Complete | StoreStatus::Running => None,
        StoreStatus::Pending | StoreStatus::Blocked => None,
    };

    if let Some(note) = note {
        return sanitize_note(&note);
    }

    // Try to get the task and extract the result's last_message
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
