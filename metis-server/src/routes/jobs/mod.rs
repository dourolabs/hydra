use crate::{
    AppState,
    state::ResolvedBundle,
    store::{Edge, Store, StoreError, Task, TaskError},
};
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use metis_common::{
    MetisId,
    artifacts::Artifact,
    constants::{ENV_GH_TOKEN, ENV_METIS_ID},
    jobs::{CreateJobRequest, CreateJobResponse, JobSummary, ListJobsResponse},
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
    let fallback_image = state.config.metis.worker_image.clone();

    let parent_ids: Vec<MetisId> = payload
        .parent_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .collect();
    if parent_ids.iter().any(|id| id.is_empty()) {
        error!("create_job received an empty parent_id");
        return Err(ApiError::bad_request("parent_ids must not be empty"));
    }
    let parent_edges: Vec<Edge> = parent_ids
        .iter()
        .map(|id| Edge {
            id: id.clone(),
            name: None,
        })
        .collect();

    // Generate a unique ID for the job
    let job_id: MetisId = uuid::Uuid::new_v4().hyphenated().to_string();

    let ResolvedBundle {
        bundle: context,
        github_token,
        default_image,
    } = state.service_state.resolve_bundle_spec(payload.context)?;
    let mut env_vars = payload.variables;
    if let Some(token) = github_token {
        env_vars.entry(ENV_GH_TOKEN.to_string()).or_insert(token);
    }
    env_vars.insert(ENV_METIS_ID.to_string(), job_id.clone());
    let image = resolve_image(payload.image, default_image, &fallback_image)?;

    // Store the task with context (status will be Pending)
    {
        let mut store = state.store.write().await;
        let task = Task::Spawn {
            program: payload.program.clone(),
            params: payload.params.clone(),
            context,
            image,
            env_vars,
        };
        store
            .add_task_with_id(job_id.clone(), task, parent_edges.clone(), Utc::now())
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

fn resolve_image(
    user_supplied: Option<String>,
    repo_default: Option<String>,
    fallback: &str,
) -> Result<String, ApiError> {
    if let Some(image) = user_supplied {
        let trimmed = image.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("image must not be empty"));
        }
        return Ok(trimmed.to_string());
    }

    if let Some(default_image) = repo_default {
        let trimmed = default_image.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let trimmed = fallback.trim();
    if trimmed.is_empty() {
        return Err(ApiError::internal(anyhow::anyhow!(
            "default worker image must not be empty"
        )));
    }

    Ok(trimmed.to_string())
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
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<JobSummary>, ApiError> {
    info!(job_id = %job_id, "get_job invoked");

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    let (summary, _) = job_summary_with_time(&job_id, store)
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

#[derive(Debug, Clone)]
pub struct JobIdPath(pub MetisId);

#[async_trait]
impl<S> FromRequestParts<S> for JobIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(job_id) = Path::<MetisId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = job_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("job_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}

async fn job_summary_with_time(
    job_id: &MetisId,
    store: &dyn Store,
) -> Result<(JobSummary, Option<DateTime<Utc>>), StoreError> {
    let status_log = store.get_status_log(job_id).await?;
    let (program, params) = match store.get_task(job_id).await? {
        Task::Spawn {
            program, params, ..
        } => (program, params),
    };
    let notes = job_notes_from_store(job_id, store).await;

    let reference_time = status_log.start_time().or(status_log.creation_time());

    Ok((
        JobSummary {
            id: job_id.clone(),
            notes,
            program,
            params,
            status_log,
        },
        reference_time,
    ))
}

async fn job_notes_from_store(job_id: &MetisId, store: &dyn Store) -> Option<String> {
    let status_log = store.get_status_log(job_id).await.ok()?;
    if let Err(err) = status_log.result()? {
        return format_error_note(&err);
    }

    let artifact_ids = status_log.emitted_artifacts()?;
    for artifact_id in artifact_ids {
        if let Ok(artifact) = store.get_artifact(&artifact_id).await {
            if let Some(note) = note_from_artifact(&artifact) {
                return Some(note);
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

fn note_from_artifact(artifact: &Artifact) -> Option<String> {
    match artifact {
        Artifact::Patch { description, .. } | Artifact::Issue { description } => {
            sanitize_note(description)
        }
    }
}
