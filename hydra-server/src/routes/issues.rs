use crate::domain::actors::{Actor, ActorRef};
use crate::routes::issue_response::{build_issue_response, build_issue_summary_response};
use crate::{
    app::{AppState, SubmitFormActionError, UpsertIssueError},
    store::{ReadOnlyStore, StoreError},
};
use anyhow::anyhow;
use axum::http::StatusCode;
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
    response::IntoResponse,
};
use hydra_common::{
    HydraId, IssueId,
    api::v1::{
        ApiError, issues as api_issues,
        pagination::{
            CursorKeys, compute_next_cursor, compute_next_cursor_with_keys, effective_limit,
        },
    },
};
use serde::Deserialize;
use tracing::{error, info};

#[derive(Debug, Deserialize)]
pub struct GetIssueQuery {
    #[serde(default)]
    pub include_archived: Option<bool>,
}

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

#[derive(Debug, Clone)]
pub struct IssueVersionPath {
    pub issue_id: IssueId,
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for IssueVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((issue_id, version)) =
            Path::<(IssueId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self { issue_id, version })
    }
}

pub async fn create_issue(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<api_issues::UpsertIssueRequest>,
) -> Result<Json<api_issues::UpsertIssueResponse>, ApiError> {
    info!("create_issue invoked");
    let (issue_id, version) = state
        .upsert_issue(None, payload, ActorRef::from(&actor))
        .await
        .map_err(map_upsert_issue_error)?;

    let api_issue = load_api_issue(&state, &issue_id).await?;
    info!(issue_id = %issue_id, "create_issue completed");
    Ok(Json(api_issues::UpsertIssueResponse::new(
        issue_id, version, api_issue,
    )))
}

pub async fn update_issue(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(payload): Json<api_issues::UpsertIssueRequest>,
) -> Result<Json<api_issues::UpsertIssueResponse>, ApiError> {
    info!(issue_id = %issue_id, "update_issue invoked");
    let (issue_id, version) = state
        .upsert_issue(Some(issue_id), payload, ActorRef::from(&actor))
        .await
        .map_err(map_upsert_issue_error)?;

    let api_issue = load_api_issue(&state, &issue_id).await?;
    info!(issue_id = %issue_id, "update_issue completed");
    Ok(Json(api_issues::UpsertIssueResponse::new(
        issue_id, version, api_issue,
    )))
}

async fn load_api_issue(
    state: &AppState,
    issue_id: &IssueId,
) -> Result<api_issues::Issue, ApiError> {
    let issue = state
        .get_issue(issue_id, true)
        .await
        .map_err(|err| map_issue_error(err, Some(issue_id)))?;
    build_issue_response(state, issue.item).await
}

pub async fn get_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Query(query): Query<GetIssueQuery>,
) -> Result<Json<api_issues::IssueVersionRecord>, ApiError> {
    let include_archived = query.include_archived.unwrap_or(false);
    info!(issue_id = %issue_id, include_archived, "get_issue invoked");
    let issue = state
        .get_issue(&issue_id, include_archived)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let object_id = HydraId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(issue_id = %issue_id, "get_issue completed");
    let api_issue = build_issue_response(&state, issue.item).await?;
    let response = api_issues::IssueVersionRecord::new(
        issue_id,
        issue.version,
        issue.timestamp,
        api_issue,
        issue.actor,
        issue.creation_time,
        labels,
    );
    Ok(Json(response))
}

pub async fn list_issue_versions(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<api_issues::ListIssueVersionsResponse>, ApiError> {
    info!(issue_id = %issue_id, "list_issue_versions invoked");
    let versions = state
        .get_issue_versions(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let object_id = HydraId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let mut records = Vec::with_capacity(versions.len());
    for version in versions {
        let api_issue = build_issue_response(&state, version.item).await?;
        records.push(api_issues::IssueVersionRecord::new(
            issue_id.clone(),
            version.version,
            version.timestamp,
            api_issue,
            version.actor,
            version.creation_time,
            labels.clone(),
        ));
    }

    let response = api_issues::ListIssueVersionsResponse::new(records);
    info!(
        issue_id = %issue_id,
        returned = response.versions.len(),
        "list_issue_versions completed"
    );
    Ok(Json(response))
}

pub async fn get_issue_version(
    State(state): State<AppState>,
    IssueVersionPath {
        issue_id,
        version: raw_version,
    }: IssueVersionPath,
) -> Result<Json<api_issues::IssueVersionRecord>, ApiError> {
    info!(issue_id = %issue_id, raw_version = raw_version.as_i64(), "get_issue_version invoked");
    let versions = state
        .get_issue_versions(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version = super::resolve_version(raw_version, max_version, "issue", issue_id.as_ref())?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!("issue '{issue_id}' version {version} not found"))
        })?;

    let object_id = HydraId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let api_issue = build_issue_response(&state, entry.item).await?;
    let response = api_issues::IssueVersionRecord::new(
        issue_id.clone(),
        entry.version,
        entry.timestamp,
        api_issue,
        entry.actor,
        entry.creation_time,
        labels,
    );
    info!(issue_id = %issue_id, version, "get_issue_version completed");
    Ok(Json(response))
}

pub async fn list_issues(
    State(state): State<AppState>,
    Query(query): Query<api_issues::SearchIssuesQuery>,
) -> Result<Json<api_issues::ListIssuesResponse>, ApiError> {
    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        project_id = ?query.project_id,
        assignee = ?query.assignee,
        creator = ?query.creator,
        query = ?query.q,
        ids_count = query.ids.len(),
        include_archived = ?query.include_archived,
        label_ids = ?query.label_ids,
        sort = ?query.sort,
        bucket_by = ?query.bucket_by,
        bucket_limit = ?query.bucket_limit,
        "list_issues invoked"
    );

    if query.bucket_by.is_some() {
        if query.cursor.is_some() {
            return Err(ApiError::bad_request(
                "cursor is incompatible with bucket_by; per-cell pagination is a single-cell unbucketed query".to_string(),
            ));
        }
        match query.bucket_limit {
            None => {
                return Err(ApiError::bad_request(
                    "bucket_by requires bucket_limit".to_string(),
                ));
            }
            Some(0) => {
                return Err(ApiError::bad_request(
                    "bucket_limit must be > 0".to_string(),
                ));
            }
            Some(_) => {}
        }
    }

    let issues = state
        .list_issues_with_query(&query)
        .await
        .map_err(|err| map_issue_error(err, None))?;

    // Batch-fetch labels for issues
    let object_ids: Vec<HydraId> = issues
        .iter()
        .map(|(id, _)| HydraId::from(id.clone()))
        .collect();

    let labels_map = state
        .get_labels_for_objects(&object_ids)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to batch-fetch labels for issues");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let eff_limit = effective_limit(query.limit);

    let mut filtered: Vec<api_issues::IssueSummaryRecord> = Vec::new();
    for (id, versioned) in issues {
        let object_id = HydraId::from(id.clone());
        let labels = labels_map.get(&object_id).cloned().unwrap_or_default();

        let summary = build_issue_summary_response(&state, &versioned.item).await?;
        let record = api_issues::IssueSummaryRecord::new(
            id.clone(),
            versioned.version,
            versioned.timestamp,
            summary,
            versioned.actor,
            versioned.creation_time,
            labels,
        );
        filtered.push(record);
    }

    // Bucketed responses are bounded by `bucket_limit * num_cells` and don't
    // paginate via cursor; per-cell "load more" is a single-cell unbucketed
    // follow-up call. Skip cursor computation entirely.
    let next_cursor = if query.bucket_by.is_some() {
        None
    } else {
        match query.sort {
            Some(api_issues::IssueSort::ProjectStatusTimeDesc) => {
                // Project priority isn't carried on the issue record; fetch it
                // once here so the cursor for the last row can encode the full
                // four-key tuple `(priority, position, created_at, id)`.
                let projects = state
                    .store
                    .list_projects(true)
                    .await
                    .map_err(|err| map_issue_error(err, None))?;
                let priority_by_project: std::collections::HashMap<_, f64> = projects
                    .iter()
                    .map(|(id, v)| (id.clone(), v.item.priority))
                    .collect();
                compute_next_cursor_with_keys(&mut filtered, eff_limit, |r| {
                    CursorKeys::ProjectStatusTime {
                        project_priority: priority_by_project
                            .get(&r.issue.project_id)
                            .copied()
                            .unwrap_or(0.0),
                        status_position: r.issue.status.position,
                        timestamp: r.timestamp,
                        id: r.issue_id.as_ref().to_string(),
                    }
                })
            }
            _ => compute_next_cursor(
                &mut filtered,
                eff_limit,
                |r| &r.timestamp,
                |r| r.issue_id.as_ref(),
            ),
        }
    };

    // Compute total_count when count=true
    let total_count = if query.count == Some(true) {
        let count = state
            .count_issues(&query)
            .await
            .map_err(|err| map_issue_error(err, None))?;
        Some(count)
    } else {
        None
    };

    let mut response = api_issues::ListIssuesResponse::new(filtered);
    response.next_cursor = next_cursor;
    response.total_count = total_count;
    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        returned = response.issues.len(),
        "list_issues completed"
    );
    Ok(Json(response))
}

fn map_upsert_issue_error(err: UpsertIssueError) -> ApiError {
    match err {
        UpsertIssueError::JobIdProvidedForUpdate => {
            ApiError::bad_request("job_id may only be provided when creating an issue")
        }
        UpsertIssueError::MissingCreator => ApiError::bad_request("issue creator must be set"),
        UpsertIssueError::UnknownAssignee { principal } => {
            ApiError::bad_request(format!("unknown actor '{principal}'"))
        }
        UpsertIssueError::AssigneeLookup { source, principal } => {
            error!(
                principal = %principal,
                error = %source,
                "failed to validate assignee existence"
            );
            ApiError::internal(anyhow!(
                "failed to validate assignee existence for '{principal}': {source}"
            ))
        }
        UpsertIssueError::MissingDependency { dependency_id, .. } => {
            ApiError::bad_request(format!("issue dependency '{dependency_id}' not found"))
        }
        UpsertIssueError::IssueNotFound { issue_id, source } => {
            map_issue_error(source, Some(&issue_id))
        }
        UpsertIssueError::Store { source, issue_id } => map_issue_error(source, issue_id.as_ref()),
        UpsertIssueError::JobNotFound { job_id, .. } => {
            error!(job_id = %job_id, "job not found when creating issue");
            ApiError::not_found(format!("job '{job_id}' not found"))
        }
        UpsertIssueError::JobStatusLookup { job_id, source } => {
            error!(job_id = %job_id, error = %source, "failed to validate job status");
            ApiError::internal(anyhow!(
                "failed to validate job status for '{job_id}': {source}"
            ))
        }
        UpsertIssueError::JobNotRunning { .. } => {
            ApiError::bad_request("job_id must reference a running job")
        }
        UpsertIssueError::TaskLookup { issue_id, source } => {
            map_issue_error(source, Some(&issue_id))
        }
        UpsertIssueError::KillTask {
            issue_id,
            job_id,
            source,
        } => ApiError::internal(anyhow!(
            "failed to kill task '{job_id}' for dropped issue '{issue_id}': {source}"
        )),
        UpsertIssueError::PolicyViolation(violation) => ApiError::bad_request(violation.message),
        UpsertIssueError::InvalidForm { message } => ApiError::bad_request(message),
    }
}

fn map_issue_error(err: StoreError, issue_id: Option<&IssueId>) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => {
            error!(issue_id = %id, "issue not found");
            ApiError::not_found(format!("issue '{id}' not found"))
        }
        StoreError::InvalidDependency(dependency_id) => {
            let issue_id = issue_id.map(|id| id.to_string()).unwrap_or_default();
            error!(
                issue_id = %issue_id,
                dependency_id = %dependency_id,
                "invalid issue dependency"
            );
            ApiError::bad_request(format!("issue dependency '{dependency_id}' not found"))
        }
        StoreError::InvalidIssueStatus(message) => {
            let issue_id = issue_id.map(|id| id.to_string()).unwrap_or_default();
            error!(
                issue_id = %issue_id,
                %message,
                "invalid issue status transition"
            );
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

pub async fn archive_issue(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<api_issues::IssueVersionRecord>, ApiError> {
    info!(issue_id = %issue_id, "archive_issue invoked");
    state
        .archive_issue(&issue_id, ActorRef::from(&actor))
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let issue = state
        .get_issue(&issue_id, true)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let object_id = HydraId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(issue_id = %issue_id, "archive_issue completed");
    let api_issue = build_issue_response(&state, issue.item).await?;
    let response = api_issues::IssueVersionRecord::new(
        issue_id,
        issue.version,
        issue.timestamp,
        api_issue,
        issue.actor,
        issue.creation_time,
        labels,
    );
    Ok(Json(response))
}

pub async fn submit_form_action(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<api_issues::SubmitFormRequest>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    info!(issue_id = %issue_id, action_id = %request.action_id, "submit_form_action invoked");

    match state
        .submit_form_action(
            issue_id.clone(),
            request.action_id,
            request.values,
            ActorRef::from(&actor),
        )
        .await
    {
        Ok((version, form_response)) => {
            info!(issue_id = %issue_id, "submit_form_action completed");
            Ok(Json(api_issues::SubmitFormResponse::new(
                issue_id,
                version,
                form_response,
            )))
        }
        Err(SubmitFormActionError::IssueNotFound { issue_id, source }) => {
            info!(issue_id = %issue_id, outcome = "issue_not_found", "submit_form_action completed");
            Err(map_issue_error(source, Some(&issue_id)).into_response())
        }
        Err(SubmitFormActionError::ActionNotFound { issue_id }) => {
            info!(issue_id = %issue_id, outcome = "action_not_found", "submit_form_action completed");
            Err(ApiError::not_found(format!(
                "issue '{issue_id}' has no form or no matching action"
            ))
            .into_response())
        }
        Err(SubmitFormActionError::ValidationFailed { field_errors }) => {
            info!(issue_id = %issue_id, outcome = "validation_failed", "submit_form_action completed");
            let body = api_issues::FormValidationError::new(field_errors);
            Err((StatusCode::BAD_REQUEST, Json(body)).into_response())
        }
        Err(SubmitFormActionError::Store { issue_id, source }) => {
            info!(issue_id = %issue_id, outcome = "store_error", "submit_form_action completed");
            Err(map_issue_error(source, Some(&issue_id)).into_response())
        }
        Err(SubmitFormActionError::UnsupportedActor { actor_name }) => {
            info!(issue_id = %issue_id, actor_name = %actor_name, outcome = "unsupported_actor", "submit_form_action completed");
            Err(
                ApiError::forbidden(format!("actor '{actor_name}' cannot submit form actions"))
                    .into_response(),
            )
        }
    }
}
