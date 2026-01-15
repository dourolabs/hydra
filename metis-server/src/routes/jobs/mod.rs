use crate::{
    app::{AppState, BundleResolutionError, CreateJobError},
    store::{Store, StoreError, TaskError, TaskResolutionError},
};
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use metis_common::{
    IssueId, TaskId,
    issues::Issue,
    jobs::{CreateJobRequest, CreateJobResponse, JobRecord, ListJobsResponse, SearchJobsQuery},
    patches::Patch,
};
use serde_json::json;
use tracing::{error, info};

pub mod context;
pub mod kill;
pub mod logs;
pub mod status;

pub async fn create_job(
    State(state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    info!("create_job invoked");
    let job_id = state.create_job(payload).await.map_err(|err| match err {
        CreateJobError::TaskResolution(err) => ApiError::from(err),
        CreateJobError::Store { source, job_id } => {
            error!(error = %source, job_id = %job_id, "failed to store task");
            ApiError::internal(anyhow::anyhow!("Failed to store task: {source}"))
        }
    })?;

    info!(
        job_id = %job_id,
        "task stored, will be started by background thread"
    );

    Ok(Json(CreateJobResponse { job_id }))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<SearchJobsQuery>,
) -> Result<Json<ListJobsResponse>, ApiError> {
    info!(
        query = ?query.q,
        spawned_from = ?query.spawned_from,
        "list_jobs invoked"
    );
    let config = state.config;
    let namespace = config.metis.namespace.clone();

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let spawned_from_filter = query.spawned_from.as_ref();

    // Get all tasks with all statuses
    let task_ids = store.list_tasks().await.map_err(|err| {
        error!(error = %err, "failed to list tasks");
        ApiError::internal(anyhow::anyhow!("Failed to list tasks: {err}"))
    })?;

    // Collect all summaries with their reference times for sorting
    let mut summaries_with_times: Vec<(JobRecord, Option<DateTime<Utc>>)> = Vec::new();
    for task_id in task_ids {
        match job_record_with_time(&task_id, store).await {
            Ok(summary) => {
                if spawned_from_matches(spawned_from_filter, &summary.0)
                    && job_matches(search_term.as_deref(), &summary.0)
                {
                    summaries_with_times.push(summary);
                }
            }
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

    let summaries: Vec<JobRecord> = summaries_with_times
        .into_iter()
        .map(|(record, _)| record)
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
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<JobRecord>, ApiError> {
    info!(job_id = %job_id, "get_job invoked");

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    let (summary, _) = job_record_with_time(&job_id, store)
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

impl From<BundleResolutionError> for ApiError {
    fn from(error: BundleResolutionError) -> Self {
        match error {
            BundleResolutionError::UnknownRepository(_) => ApiError::bad_request(error.to_string()),
        }
    }
}

impl From<TaskResolutionError> for ApiError {
    fn from(error: TaskResolutionError) -> Self {
        match error {
            TaskResolutionError::EmptyImage => ApiError::bad_request(error.to_string()),
            TaskResolutionError::Bundle(err) => ApiError::from(err),
            TaskResolutionError::MissingDefaultImage => ApiError::internal(error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

#[derive(Debug, Clone)]
pub struct JobIdPath(pub TaskId);

#[async_trait]
impl<S> FromRequestParts<S> for JobIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(job_id) = Path::<TaskId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(job_id))
    }
}

async fn job_record_with_time(
    job_id: &TaskId,
    store: &dyn Store,
) -> Result<(JobRecord, Option<DateTime<Utc>>), StoreError> {
    let status_log = store.get_status_log(job_id).await?;
    let task = store.get_task(job_id).await?;
    let notes = job_notes_from_store(job_id, store).await;

    let reference_time = status_log.start_time().or(status_log.creation_time());

    Ok((
        JobRecord {
            id: job_id.clone(),
            task,
            notes,
            status_log,
        },
        reference_time,
    ))
}

fn spawned_from_matches(expected: Option<&IssueId>, job: &JobRecord) -> bool {
    match expected {
        Some(issue_id) => job.task.spawned_from.as_ref() == Some(issue_id),
        None => true,
    }
}

fn job_matches(search_term: Option<&str>, job: &JobRecord) -> bool {
    if let Some(term) = search_term {
        let lower_term = term.to_lowercase();
        let contains = |value: &str| value.to_lowercase().contains(&lower_term);

        if contains(job.id.as_ref()) || contains(&job.task.prompt) {
            return true;
        }

        if let Some(note) = &job.notes {
            if contains(note) {
                return true;
            }
        }

        return contains(&format!("{:?}", job.status_log.current_status()));
    }

    true
}

async fn job_notes_from_store(job_id: &TaskId, store: &dyn Store) -> Option<String> {
    let status_log = store.get_status_log(job_id).await.ok()?;
    if let Err(err) = status_log.result()? {
        return format_error_note(&err);
    }

    let artifact_ids = status_log.emitted_artifacts()?;
    for artifact_id in artifact_ids {
        if let Some(patch_id) = artifact_id.as_patch_id() {
            if let Ok(patch) = store.get_patch(&patch_id).await {
                if let Some(note) = note_from_patch(&patch) {
                    return Some(note);
                }
            }
        }

        if let Some(issue_id) = artifact_id.as_issue_id() {
            if let Ok(issue) = store.get_issue(&issue_id).await {
                if let Some(note) = note_from_issue(&issue) {
                    return Some(note);
                }
            }
        }
    }

    None
}

pub(crate) fn sanitize_note(note: &str) -> Option<String> {
    let collapsed = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn format_error_note(error: &TaskError) -> Option<String> {
    match error {
        TaskError::JobEngineError { reason } => {
            sanitize_note(reason).map(|msg| format!("error: {msg}"))
        }
    }
}

fn note_from_patch(patch: &Patch) -> Option<String> {
    sanitize_note(&patch.title).or_else(|| sanitize_note(&patch.description))
}

fn note_from_issue(issue: &Issue) -> Option<String> {
    sanitize_note(&issue.description)
}
