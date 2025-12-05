use crate::{
    AppState,
    job_engine::{JobStatus, JobEngineError},
    store::{Store, Task},
};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Duration as ChronoDuration, Utc};
use metis_common::{
    jobs::{CreateJobRequest, CreateJobResponse, JobSummary, ListJobsResponse},
};
use serde_json::json;
use tracing::{error, info};

pub mod logs;
pub mod output;
pub mod context;
pub mod kill;

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

    // Generate a unique ID for the job
    let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

    // Store the task with context and prompt (status will be Pending)
    {
        let mut store = state.store.write().await;
        let task = Task::Spawn {
            prompt: prompt.clone(),
            context: payload.context.clone(),
            result: None,
        };
        store.add_task_with_id(job_id.clone(), task, vec![]).await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to store task");
                ApiError::internal(anyhow::anyhow!("Failed to store task: {}", err))
            })?;
    }

    info!(job_id = %job_id, "task stored, will be started by background thread");

    Ok(Json(CreateJobResponse {
        job_id,
    }))
}

pub async fn list_jobs(State(state): State<AppState>) -> Result<Json<ListJobsResponse>, ApiError> {
    info!("list_jobs invoked");
    let config = state.config;
    let namespace = config.metis.namespace.clone();
    
    let metis_jobs = state.job_engine.list_jobs().await
        .map_err(|err| {
            error!(error = ?err, namespace = %namespace, "failed to list jobs");
            match err {
                JobEngineError::Kubernetes(kube_err) => {
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
    
    let mut summaries = Vec::new();
    for job in metis_jobs {
        let runtime = job_runtime(&job, now).map(format_duration);
        let notes = {
            let store_read = state.store.read().await;
            job_notes(&job.id, job.status, &job.failure_message, store_read.as_ref()).await
        };

        summaries.push(JobSummary {
            id: job.id,
            status: job.status.to_string(),
            runtime,
            notes,
        });
    }

    info!(
        namespace = %namespace,
        job_count = summaries.len(),
        "list_jobs completed successfully"
    );

    Ok(Json(ListJobsResponse {
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


fn job_runtime(job: &crate::job_engine::MetisJob, now: chrono::DateTime<Utc>) -> Option<ChronoDuration> {
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

async fn job_notes(
    job_id: &str,
    status: JobStatus,
    failure_message: &Option<String>,
    store: &dyn Store,
) -> Option<String> {
    let note = match status {
        JobStatus::Failed => {
            failure_message.clone().or_else(|| {
                None
            })
        }
        JobStatus::Complete | JobStatus::Running => {
            None
        }
    };

    if let Some(note) = note {
        return sanitize_note(&note);
    }

    // Try to get the task and extract the result's last_message
    let job_id_string = job_id.to_string();
    if let Ok(task) = store.get_task(&job_id_string).await {
        if let Task::Spawn { result: Some(output), .. } = task {
            return sanitize_note(&output.last_message);
        }
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
