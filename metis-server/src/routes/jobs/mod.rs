use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{AppState, BundleResolutionError, CreateJobError, TaskResolutionError},
    store::StoreError,
};
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{TaskId, api::v1};
use tracing::{error, info};

pub use metis_common::api::v1::ApiError;

pub mod context;
pub mod kill;
pub mod logs;
pub mod status;

pub async fn create_job(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<v1::jobs::CreateJobRequest>,
) -> Result<Json<v1::jobs::CreateJobResponse>, ApiError> {
    info!("create_job invoked");
    let job_id = state
        .create_job(payload, ActorRef::from(&actor), actor.creator.clone())
        .await
        .map_err(|err| match err {
            CreateJobError::TaskResolution(err) => ApiError::from(err),
            CreateJobError::IssueLookup { source, issue_id } => match source {
                StoreError::IssueNotFound(_) => {
                    ApiError::not_found(format!("issue '{issue_id}' not found"))
                }
                other => {
                    error!(
                        error = %other,
                        issue_id = %issue_id,
                        "failed to load issue for job creation"
                    );
                    ApiError::internal(format!("Failed to load issue '{issue_id}': {other}"))
                }
            },
            CreateJobError::Store { source } => {
                error!(error = %source, "failed to store task");
                ApiError::internal(format!("Failed to store task: {source}"))
            }
        })?;

    info!(
        job_id = %job_id,
        "task stored, will be started by background thread"
    );

    Ok(Json(v1::jobs::CreateJobResponse::new(job_id)))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<v1::jobs::SearchJobsQuery>,
) -> Result<Json<v1::jobs::ListJobsResponse>, ApiError> {
    info!(
        query = ?query.q,
        spawned_from = ?query.spawned_from,
        include_deleted = ?query.include_deleted,
        "list_jobs invoked"
    );
    let namespace = state.config.metis.namespace.clone();

    // All filtering (q, spawned_from, include_deleted) is done at the Store level.
    // Text search (q) matches task ID, prompt, and status (NOT notes).
    let tasks = state.list_tasks_with_query(&query).await.map_err(|err| {
        error!(error = %err, "failed to list tasks");
        ApiError::internal(format!("Failed to list tasks: {err}"))
    })?;

    // Timing fields (creation_time, start_time, end_time) are denormalized
    // on the task and flow through the domain→API conversion automatically.
    let mut summaries: Vec<v1::jobs::JobSummaryRecord> = tasks
        .into_iter()
        .map(|(task_id, versioned_task)| {
            let api_task: v1::jobs::Task = versioned_task.item.into();
            let full_record = v1::jobs::JobVersionRecord::new(
                task_id,
                versioned_task.version,
                versioned_task.timestamp,
                api_task,
                versioned_task.actor,
            );
            v1::jobs::JobSummaryRecord::from(&full_record)
        })
        .collect();

    // Sort by version timestamp, most recent first
    summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    info!(
        namespace = %namespace,
        job_count = summaries.len(),
        "list_jobs completed successfully"
    );

    let response = v1::jobs::ListJobsResponse::new(summaries);
    Ok(Json(response))
}

pub async fn get_job(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<v1::jobs::JobVersionRecord>, ApiError> {
    info!(job_id = %job_id, "get_job invoked");

    let versions = state
        .get_task_versions(&job_id)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) => {
                error!(job_id = %job_id, "job not found");
                ApiError::not_found(format!("job '{job_id}' not found"))
            }
            err => {
                error!(job_id = %job_id, error = %err, "failed to load job");
                ApiError::internal(format!("Failed to load job '{job_id}': {err}"))
            }
        })?;

    let latest = versions.last().ok_or_else(|| {
        error!(job_id = %job_id, "job has no versions");
        ApiError::not_found(format!("job '{job_id}' not found"))
    })?;

    let status_log = crate::store::task_status_log_from_versions(&versions);
    let mut api_task: v1::jobs::Task = latest.item.clone().into();
    if let Some(log) = &status_log {
        api_task.creation_time = log.creation_time();
        api_task.start_time = log.start_time();
        api_task.end_time = log.end_time();
    }
    let record = v1::jobs::JobVersionRecord::new(
        job_id.clone(),
        latest.version,
        latest.timestamp,
        api_task,
        latest.actor.clone(),
    );
    info!(job_id = %record.job_id, "get_job completed successfully");
    Ok(Json(record))
}

pub async fn list_job_versions(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<v1::jobs::ListJobVersionsResponse>, ApiError> {
    info!(job_id = %job_id, "list_job_versions invoked");
    let versions = state
        .get_task_versions(&job_id)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) => {
                error!(job_id = %job_id, "job not found");
                ApiError::not_found(format!("job '{job_id}' not found"))
            }
            other => {
                error!(job_id = %job_id, error = %other, "failed to load job versions");
                ApiError::internal(format!("Failed to load job '{job_id}': {other}"))
            }
        })?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::jobs::JobVersionRecord::new(
                job_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
                version.actor,
            )
        })
        .collect();

    let response = v1::jobs::ListJobVersionsResponse::new(records);
    info!(
        job_id = %job_id,
        returned = response.versions.len(),
        "list_job_versions completed"
    );
    Ok(Json(response))
}

pub async fn get_job_version(
    State(state): State<AppState>,
    JobVersionPath {
        job_id,
        version: raw_version,
    }: JobVersionPath,
) -> Result<Json<v1::jobs::JobVersionRecord>, ApiError> {
    info!(job_id = %job_id, raw_version = raw_version.as_i64(), "get_job_version invoked");
    let versions = state
        .get_task_versions(&job_id)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) => {
                error!(job_id = %job_id, "job not found");
                ApiError::not_found(format!("job '{job_id}' not found"))
            }
            other => {
                error!(job_id = %job_id, error = %other, "failed to load job versions");
                ApiError::internal(format!("Failed to load job '{job_id}': {other}"))
            }
        })?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version = super::resolve_version(raw_version, max_version, "job", job_id.as_ref())?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!("job '{job_id}' version {version} not found"))
        })?;

    let response = v1::jobs::JobVersionRecord::new(
        job_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
        entry.actor,
    );
    info!(job_id = %job_id, version, "get_job_version completed");
    Ok(Json(response))
}

impl From<BundleResolutionError> for ApiError {
    fn from(error: BundleResolutionError) -> Self {
        match error {
            BundleResolutionError::UnknownRepository(_)
            | BundleResolutionError::UnsupportedBundleSpec => {
                ApiError::bad_request(error.to_string())
            }
            BundleResolutionError::RepositoryLookup { .. } => ApiError::internal(error.to_string()),
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

#[derive(Debug, Clone)]
pub struct JobVersionPath {
    pub job_id: TaskId,
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for JobVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((job_id, version)) =
            Path::<(TaskId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self { job_id, version })
    }
}
