use crate::{
    app::{AppState, BundleResolutionError, CreateJobError, TaskResolutionError},
    store::{Store, StoreError, TaskError},
};
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::{DateTime, Utc};
use metis_common::{
    IssueId, TaskId,
    api::v1::jobs::{CreateJobRequest, CreateJobResponse, JobRecord, ListJobsResponse, SearchJobsQuery},
};
use tracing::{error, info};

pub use metis_common::api::v1::ApiError;

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
            ApiError::internal(format!("Failed to store task: {source}"))
        }
    })?;

    info!(
        job_id = %job_id,
        "task stored, will be started by background thread"
    );

    Ok(Json(CreateJobResponse::new(job_id)))
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
        ApiError::internal(format!("Failed to list tasks: {err}"))
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

    Ok(Json(ListJobsResponse::new(summaries)))
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
                ApiError::internal(format!("Failed to load job '{job_id}': {err}"))
            }
        })?;

    info!(job_id = %summary.id, "get_job completed successfully");
    Ok(Json(summary))
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
            TaskResolutionError::MissingDefaultImage => ApiError::internal(error.to_string()),
        }
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

    Ok((JobRecord::new(job_id.clone(), task, notes, status_log), reference_time))
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
    status_log
        .result()
        .and_then(Result::err)
        .and_then(format_error_note)
}

pub(crate) fn sanitize_note(note: &str) -> Option<String> {
    let collapsed = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn format_error_note(error: TaskError) -> Option<String> {
    match error {
        TaskError::JobEngineError { reason } => {
            sanitize_note(&reason).map(|msg| format!("error: {msg}"))
        }
    }
}
