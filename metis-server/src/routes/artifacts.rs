use crate::{
    AppState,
    routes::jobs::ApiError,
    store::{Status, Store, StoreError},
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::Utc;
use metis_common::artifacts::{
    Issue, IssueDependency, IssueRecord, IssueStatus, IssueType, ListIssuesResponse,
    ListPatchesResponse, Patch, PatchRecord, SearchIssuesQuery, SearchPatchesQuery,
    UpsertIssueRequest, UpsertIssueResponse, UpsertPatchRequest, UpsertPatchResponse,
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct IssueIdPath(pub String);

#[derive(Debug, Clone)]
pub struct PatchIdPath(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for IssueIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(issue_id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = issue_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("issue_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for PatchIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(patch_id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = patch_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("patch_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}

pub async fn create_issue(
    State(state): State<AppState>,
    Json(payload): Json<UpsertIssueRequest>,
) -> Result<Json<UpsertIssueResponse>, ApiError> {
    info!("create_issue invoked");
    upsert_issue_internal(state, None, payload).await
}

pub async fn update_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(payload): Json<UpsertIssueRequest>,
) -> Result<Json<UpsertIssueResponse>, ApiError> {
    info!(issue_id = %issue_id, "update_issue invoked");
    upsert_issue_internal(state, Some(issue_id), payload).await
}

pub async fn get_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<IssueRecord>, ApiError> {
    info!(issue_id = %issue_id, "get_issue invoked");
    let store_read = state.store.read().await;
    let issue = store_read
        .get_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    Ok(Json(IssueRecord {
        id: issue_id,
        issue,
    }))
}

pub async fn list_issues(
    State(state): State<AppState>,
    Query(query): Query<SearchIssuesQuery>,
) -> Result<Json<ListIssuesResponse>, ApiError> {
    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        query = ?query.q,
        "list_issues invoked"
    );

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let assignee_filter = query
        .assignee
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let issues = store_read
        .list_issues()
        .await
        .map_err(|err| map_issue_error(err, None))?;

    let filtered = issues
        .into_iter()
        .filter(|(id, issue)| {
            issue_matches(
                query.issue_type,
                query.status,
                search_term.as_deref(),
                assignee_filter,
                id,
                issue,
            )
        })
        .map(|(id, issue)| IssueRecord { id, issue })
        .collect();

    Ok(Json(ListIssuesResponse { issues: filtered }))
}

pub async fn create_patch(
    State(state): State<AppState>,
    Json(payload): Json<UpsertPatchRequest>,
) -> Result<Json<UpsertPatchResponse>, ApiError> {
    info!("create_patch invoked");
    upsert_patch_internal(state, None, payload).await
}

pub async fn update_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
    Json(payload): Json<UpsertPatchRequest>,
) -> Result<Json<UpsertPatchResponse>, ApiError> {
    info!(patch_id = %patch_id, "update_patch invoked");
    upsert_patch_internal(state, Some(patch_id), payload).await
}

pub async fn get_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<PatchRecord>, ApiError> {
    info!(patch_id = %patch_id, "get_patch invoked");
    let store_read = state.store.read().await;
    let patch = store_read
        .get_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    Ok(Json(PatchRecord {
        id: patch_id,
        patch,
    }))
}

pub async fn list_patches(
    State(state): State<AppState>,
    Query(query): Query<SearchPatchesQuery>,
) -> Result<Json<ListPatchesResponse>, ApiError> {
    info!(query = ?query.q, "list_patches invoked");

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let patches = store_read
        .list_patches()
        .await
        .map_err(|err| map_patch_error(err, None))?;

    let filtered = patches
        .into_iter()
        .filter(|(id, patch)| patch_matches(search_term.as_deref(), id, patch))
        .map(|(id, patch)| PatchRecord { id, patch })
        .collect();

    Ok(Json(ListPatchesResponse { patches: filtered }))
}

async fn validate_issue_dependencies(
    store: &mut dyn Store,
    dependencies: &[IssueDependency],
) -> Result<(), ApiError> {
    for dependency in dependencies {
        let target_id = &dependency.issue_id;
        store.get_issue(target_id).await.map_err(|err| match err {
            StoreError::IssueNotFound(id) => {
                ApiError::bad_request(format!("issue dependency '{id}' not found"))
            }
            other => map_issue_error(other, Some(target_id)),
        })?;
    }

    Ok(())
}

async fn upsert_issue_internal(
    state: AppState,
    issue_id: Option<String>,
    payload: UpsertIssueRequest,
) -> Result<Json<UpsertIssueResponse>, ApiError> {
    let UpsertIssueRequest { issue, job_id } = payload;

    let mut store = state.store.write().await;
    validate_issue_dependencies(store.as_mut(), &issue.dependencies).await?;

    let issue_id = match issue_id {
        Some(id) => {
            if job_id.is_some() {
                return Err(ApiError::bad_request(
                    "job_id may only be provided when creating an issue",
                ));
            }

            match store.update_issue(&id, issue).await {
                Ok(()) => id,
                Err(err) => return Err(map_issue_error(err, Some(&id))),
            }
        }
        None => {
            let job_id = job_id
                .as_ref()
                .map(|value| value.trim())
                .map(|value| value.to_string());

            if let Some(ref job_id) = job_id {
                if job_id.is_empty() {
                    return Err(ApiError::bad_request("job_id must not be empty"));
                }

                let status = store.get_status(job_id).await.map_err(|err| match err {
                    StoreError::TaskNotFound(id) => {
                        error!(job_id = %id, "job not found when creating issue");
                        ApiError::not_found(format!("job '{id}' not found"))
                    }
                    other => {
                        error!(job_id = %job_id, error = %other, "failed to validate job status");
                        ApiError::internal(anyhow!(
                            "failed to validate job status for '{job_id}': {other}"
                        ))
                    }
                })?;

                if status != Status::Running {
                    return Err(ApiError::bad_request(
                        "job_id must reference a running job to record emitted artifacts",
                    ));
                }
            }

            let id = store
                .add_issue(issue)
                .await
                .map_err(|err| map_issue_error(err, None))?;

            if let Some(job_id) = job_id {
                store
                    .emit_task_artifacts(&job_id, vec![id.clone()], Utc::now())
                    .await
                    .map_err(|err| map_emit_error(err, &job_id))?;
            }

            id
        }
    };

    info!(issue_id = %issue_id, "issue stored successfully");

    Ok(Json(UpsertIssueResponse { issue_id }))
}

async fn upsert_patch_internal(
    state: AppState,
    patch_id: Option<String>,
    payload: UpsertPatchRequest,
) -> Result<Json<UpsertPatchResponse>, ApiError> {
    let UpsertPatchRequest { patch, job_id } = payload;

    let mut store = state.store.write().await;
    let patch_id = match patch_id {
        Some(id) => {
            if job_id.is_some() {
                return Err(ApiError::bad_request(
                    "job_id may only be provided when creating a patch",
                ));
            }

            match store.update_patch(&id, patch).await {
                Ok(()) => id,
                Err(err) => return Err(map_patch_error(err, Some(&id))),
            }
        }
        None => {
            let job_id = job_id
                .as_ref()
                .map(|value| value.trim())
                .map(|value| value.to_string());

            if let Some(ref job_id) = job_id {
                if job_id.is_empty() {
                    return Err(ApiError::bad_request("job_id must not be empty"));
                }

                let status = store.get_status(job_id).await.map_err(|err| match err {
                    StoreError::TaskNotFound(id) => {
                        error!(job_id = %id, "job not found when creating patch");
                        ApiError::not_found(format!("job '{id}' not found"))
                    }
                    other => {
                        error!(job_id = %job_id, error = %other, "failed to validate job status");
                        ApiError::internal(anyhow!(
                            "failed to validate job status for '{job_id}': {other}"
                        ))
                    }
                })?;

                if status != Status::Running {
                    return Err(ApiError::bad_request(
                        "job_id must reference a running job to record emitted artifacts",
                    ));
                }
            }

            let id = store
                .add_patch(patch)
                .await
                .map_err(|err| map_patch_error(err, None))?;

            if let Some(job_id) = job_id {
                store
                    .emit_task_artifacts(&job_id, vec![id.clone()], Utc::now())
                    .await
                    .map_err(|err| map_emit_error(err, &job_id))?;
            }

            id
        }
    };

    info!(patch_id = %patch_id, "patch stored successfully");

    Ok(Json(UpsertPatchResponse { patch_id }))
}

fn issue_matches(
    issue_type_filter: Option<IssueType>,
    status_filter: Option<IssueStatus>,
    search_term: Option<&str>,
    assignee_filter: Option<&str>,
    issue_id: &str,
    issue: &Issue,
) -> bool {
    if let Some(issue_type) = issue_type_filter {
        if issue.issue_type != issue_type {
            return false;
        }
    }

    if let Some(status) = status_filter {
        if issue.status != status {
            return false;
        }
    }

    if let Some(expected_assignee) = assignee_filter {
        match issue.assignee.as_ref() {
            Some(current) if current.eq_ignore_ascii_case(expected_assignee) => {}
            _ => return false,
        }
    }

    if let Some(term) = search_term {
        let lower_id = issue_id.to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return issue.description.to_lowercase().contains(term)
            || issue.issue_type.as_str() == term
            || issue.status.as_str() == term
            || issue
                .assignee
                .as_deref()
                .map(|value| value.to_lowercase().contains(term))
                .unwrap_or(false);
    }

    true
}

fn patch_matches(search_term: Option<&str>, patch_id: &str, patch: &Patch) -> bool {
    if let Some(term) = search_term {
        let lower_id = patch_id.to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return patch.title.to_lowercase().contains(term)
            || patch.diff.to_lowercase().contains(term)
            || patch.description.to_lowercase().contains(term);
    }

    true
}

fn map_issue_error(err: StoreError, issue_id: Option<&str>) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => {
            error!(issue_id = %id, "issue not found");
            ApiError::not_found(format!("issue '{id}' not found"))
        }
        StoreError::InvalidDependency(message) => {
            error!(issue_id = issue_id.unwrap_or_default(), %message, "invalid issue dependency");
            ApiError::bad_request(message)
        }
        other => {
            error!(
                issue_id = issue_id.unwrap_or_default(),
                error = %other,
                "issue store operation failed"
            );
            ApiError::internal(anyhow!("issue store error: {other}"))
        }
    }
}

fn map_patch_error(err: StoreError, patch_id: Option<&str>) -> ApiError {
    match err {
        StoreError::PatchNotFound(id) => {
            error!(patch_id = %id, "patch not found");
            ApiError::not_found(format!("patch '{id}' not found"))
        }
        other => {
            error!(
                patch_id = patch_id.unwrap_or_default(),
                error = %other,
                "patch store operation failed"
            );
            ApiError::internal(anyhow!("patch store error: {other}"))
        }
    }
}

fn map_emit_error(err: StoreError, job_id: &str) -> ApiError {
    match err {
        StoreError::TaskNotFound(id) => {
            error!(job_id = %id, "job not found when emitting artifacts");
            ApiError::not_found(format!("job '{id}' not found"))
        }
        StoreError::InvalidStatusTransition => {
            error!(job_id = %job_id, "job not running when emitting artifacts");
            ApiError::bad_request("job must be running to record emitted artifacts")
        }
        other => {
            error!(job_id = %job_id, error = %other, "failed to emit artifacts");
            ApiError::internal(anyhow!("failed to emit artifacts for '{job_id}': {other}"))
        }
    }
}
