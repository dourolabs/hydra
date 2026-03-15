use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{AppState, BundleResolutionError, CreateSessionError, TaskResolutionError},
    store::StoreError,
};
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{
    SessionId,
    api::v1,
    api::v1::pagination::{compute_next_cursor, effective_limit},
};
use tracing::{error, info};

pub use metis_common::api::v1::ApiError;

pub mod context;
pub mod kill;
pub mod logs;
pub mod status;

pub async fn create_session(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<v1::sessions::CreateSessionRequest>,
) -> Result<Json<v1::sessions::CreateSessionResponse>, ApiError> {
    info!("create_session invoked");
    let session_id = state
        .create_session(payload, ActorRef::from(&actor), actor.creator.clone())
        .await
        .map_err(|err| match err {
            CreateSessionError::TaskResolution(err) => ApiError::from(err),
            CreateSessionError::IssueLookup { source, issue_id } => match source {
                StoreError::IssueNotFound(_) => {
                    ApiError::not_found(format!("issue '{issue_id}' not found"))
                }
                other => {
                    error!(
                        error = %other,
                        issue_id = %issue_id,
                        "failed to load issue for session creation"
                    );
                    ApiError::internal(format!("Failed to load issue '{issue_id}': {other}"))
                }
            },
            CreateSessionError::Store { source } => {
                error!(error = %source, "failed to store task");
                ApiError::internal(format!("Failed to store task: {source}"))
            }
        })?;

    info!(
        session_id = %session_id,
        "task stored, will be started by background thread"
    );

    Ok(Json(v1::sessions::CreateSessionResponse::new(session_id)))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<v1::sessions::SearchSessionsQuery>,
) -> Result<Json<v1::sessions::ListSessionsResponse>, ApiError> {
    info!(
        query = ?query.q,
        spawned_from = ?query.spawned_from,
        spawned_from_ids_count = query.spawned_from_ids.len(),
        include_deleted = ?query.include_deleted,
        "list_sessions invoked"
    );
    let namespace = state.config.metis.namespace.clone();

    // All filtering (q, spawned_from, include_deleted) is done at the Store level.
    // Text search (q) matches task ID, prompt, and status (NOT notes).
    let tasks = state
        .list_sessions_with_query(&query)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to list tasks");
            ApiError::internal(format!("Failed to list tasks: {err}"))
        })?;

    // Timing fields (creation_time, start_time, end_time) are denormalized
    // on the task and flow through the domain→API conversion automatically.
    let mut summaries: Vec<v1::sessions::SessionSummaryRecord> = tasks
        .into_iter()
        .map(|(task_id, versioned_task)| {
            let api_task: v1::sessions::Session = versioned_task.item.into();
            let full_record = v1::sessions::SessionVersionRecord::new(
                task_id,
                versioned_task.version,
                versioned_task.timestamp,
                api_task,
                versioned_task.actor,
            );
            v1::sessions::SessionSummaryRecord::from(&full_record)
        })
        .collect();

    // The store already sorts by timestamp DESC when pagination is active.
    // When no limit is set, sort client-side for backward compat.
    let eff_limit = effective_limit(query.limit);
    if eff_limit.is_none() {
        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    }

    let next_cursor = compute_next_cursor(
        &mut summaries,
        eff_limit,
        |r| &r.timestamp,
        |r| r.session_id.as_ref(),
    );

    info!(
        namespace = %namespace,
        session_count = summaries.len(),
        "list_sessions completed successfully"
    );

    let total_count = if query.count == Some(true) {
        let count = state.count_sessions(&query).await.map_err(|err| {
            error!(error = %err, "failed to count tasks");
            ApiError::internal(format!("Failed to count tasks: {err}"))
        })?;
        Some(count)
    } else {
        None
    };

    let mut response = v1::sessions::ListSessionsResponse::new(summaries);
    response.next_cursor = next_cursor;
    response.total_count = total_count;
    Ok(Json(response))
}

pub async fn get_session(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::SessionVersionRecord>, ApiError> {
    info!(session_id = %session_id, "get_session invoked");

    let versions = state
        .get_session_versions(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                error!(session_id = %session_id, "session not found");
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            err => {
                error!(session_id = %session_id, error = %err, "failed to load session");
                ApiError::internal(format!("Failed to load session '{session_id}': {err}"))
            }
        })?;

    let latest = versions.last().ok_or_else(|| {
        error!(session_id = %session_id, "session has no versions");
        ApiError::not_found(format!("session '{session_id}' not found"))
    })?;

    let status_log = crate::store::session_status_log_from_versions(&versions);
    let mut api_task: v1::sessions::Session = latest.item.clone().into();
    if let Some(log) = &status_log {
        api_task.creation_time = log.creation_time();
        api_task.start_time = log.start_time();
        api_task.end_time = log.end_time();
    }
    let record = v1::sessions::SessionVersionRecord::new(
        session_id.clone(),
        latest.version,
        latest.timestamp,
        api_task,
        latest.actor.clone(),
    );
    info!(session_id = %record.session_id, "get_session completed successfully");
    Ok(Json(record))
}

pub async fn list_session_versions(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::ListSessionVersionsResponse>, ApiError> {
    info!(session_id = %session_id, "list_session_versions invoked");
    let versions = state
        .get_session_versions(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                error!(session_id = %session_id, "session not found");
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => {
                error!(session_id = %session_id, error = %other, "failed to load session versions");
                ApiError::internal(format!("Failed to load session '{session_id}': {other}"))
            }
        })?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::sessions::SessionVersionRecord::new(
                session_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
                version.actor,
            )
        })
        .collect();

    let response = v1::sessions::ListSessionVersionsResponse::new(records);
    info!(
        session_id = %session_id,
        returned = response.versions.len(),
        "list_session_versions completed"
    );
    Ok(Json(response))
}

pub async fn get_session_version(
    State(state): State<AppState>,
    SessionVersionPath {
        session_id,
        version: raw_version,
    }: SessionVersionPath,
) -> Result<Json<v1::sessions::SessionVersionRecord>, ApiError> {
    info!(session_id = %session_id, raw_version = raw_version.as_i64(), "get_session_version invoked");
    let versions = state
        .get_session_versions(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                error!(session_id = %session_id, "session not found");
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => {
                error!(session_id = %session_id, error = %other, "failed to load session versions");
                ApiError::internal(format!("Failed to load session '{session_id}': {other}"))
            }
        })?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version = super::resolve_version(raw_version, max_version, "session", session_id.as_ref())?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "session '{session_id}' version {version} not found"
            ))
        })?;

    let response = v1::sessions::SessionVersionRecord::new(
        session_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
        entry.actor,
    );
    info!(session_id = %session_id, version, "get_session_version completed");
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
pub struct SessionIdPath(pub SessionId);

#[async_trait]
impl<S> FromRequestParts<S> for SessionIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(session_id) = Path::<SessionId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(session_id))
    }
}

#[derive(Debug, Clone)]
pub struct SessionVersionPath {
    pub session_id: SessionId,
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for SessionVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((session_id, version)) =
            Path::<(SessionId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self {
            session_id,
            version,
        })
    }
}
