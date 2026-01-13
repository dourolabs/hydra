use crate::{
    AppState,
    routes::jobs::ApiError,
    routes::map_emit_error,
    store::{Status, Store, StoreError},
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::Utc;
use metis_common::{
    MetisId,
    issues::{
        Issue, IssueDependency, IssueId, IssueRecord, IssueStatus, IssueType, ListIssuesResponse,
        SearchIssuesQuery, UpsertIssueRequest, UpsertIssueResponse,
    },
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct IssueIdPath(pub IssueId);

#[async_trait]
impl<S> FromRequestParts<S> for IssueIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(issue_id) = Path::<IssueId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(issue_id))
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
    issue_id: Option<IssueId>,
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
            if let Some(ref job_id) = job_id {
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
                    .emit_task_artifacts(&job_id, vec![MetisId::from(id.clone())], Utc::now())
                    .await
                    .map_err(|err| map_emit_error(err, job_id.as_ref()))?;
            }

            id
        }
    };

    info!(issue_id = %issue_id, "issue stored successfully");

    Ok(Json(UpsertIssueResponse { issue_id }))
}

fn issue_matches(
    issue_type_filter: Option<IssueType>,
    status_filter: Option<IssueStatus>,
    search_term: Option<&str>,
    assignee_filter: Option<&str>,
    issue_id: &IssueId,
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
        let lower_id = issue_id.to_string().to_lowercase();
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

fn map_issue_error(err: StoreError, issue_id: Option<&IssueId>) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => {
            error!(issue_id = %id, "issue not found");
            ApiError::not_found(format!("issue '{id}' not found"))
        }
        StoreError::InvalidDependency(message) => {
            let issue_id = issue_id.map(|id| id.to_string()).unwrap_or_default();
            error!(issue_id = %issue_id, %message, "invalid issue dependency");
            ApiError::bad_request(message)
        }
        other => {
            let issue_id = issue_id.map(|id| id.to_string()).unwrap_or_default();
            error!(
                issue_id = %issue_id,
                error = %other,
                "issue store operation failed"
            );
            ApiError::internal(anyhow!("issue store error: {other}"))
        }
    }
}
