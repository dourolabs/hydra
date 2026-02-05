use crate::domain::issues::{
    AddTodoItemRequest, Issue, IssueRecord, IssueStatus, IssueType, ListIssuesResponse,
    ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest, TodoItem,
    TodoListResponse, UpsertIssueRequest,
};
use crate::{
    app::{AppState, UpdateTodoListError, UpsertIssueError},
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{
    IssueId, VersionNumber,
    api::v1::{self, ApiError},
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

#[derive(Debug, Clone)]
pub struct IssueVersionPath {
    pub issue_id: IssueId,
    pub version: VersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for IssueVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((issue_id, version)) =
            Path::<(IssueId, VersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self { issue_id, version })
    }
}

#[derive(Debug, Clone)]
pub struct TodoItemPath {
    pub issue_id: IssueId,
    pub item_number: usize,
}

#[async_trait]
impl<S> FromRequestParts<S> for TodoItemPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((issue_id, item_number)) =
            Path::<(IssueId, usize)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self {
            issue_id,
            item_number,
        })
    }
}

pub async fn create_issue(
    State(state): State<AppState>,
    Json(payload): Json<v1::issues::UpsertIssueRequest>,
) -> Result<Json<v1::issues::UpsertIssueResponse>, ApiError> {
    info!("create_issue invoked");
    let request: UpsertIssueRequest = payload.into();
    let issue_id = state
        .upsert_issue(None, request)
        .await
        .map_err(map_upsert_issue_error)?;

    info!(issue_id = %issue_id, "create_issue completed");
    Ok(Json(v1::issues::UpsertIssueResponse::new(issue_id)))
}

pub async fn update_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(payload): Json<v1::issues::UpsertIssueRequest>,
) -> Result<Json<v1::issues::UpsertIssueResponse>, ApiError> {
    info!(issue_id = %issue_id, "update_issue invoked");
    let request: UpsertIssueRequest = payload.into();
    let issue_id = state
        .upsert_issue(Some(issue_id), request)
        .await
        .map_err(map_upsert_issue_error)?;

    info!(issue_id = %issue_id, "update_issue completed");
    Ok(Json(v1::issues::UpsertIssueResponse::new(issue_id)))
}

pub async fn get_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<v1::issues::IssueRecord>, ApiError> {
    info!(issue_id = %issue_id, "get_issue invoked");
    let issue = state
        .get_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    info!(issue_id = %issue_id, "get_issue completed");
    let response: v1::issues::IssueRecord = IssueRecord::new(issue_id, issue.item).into();
    Ok(Json(response))
}

pub async fn list_issue_versions(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<v1::issues::ListIssueVersionsResponse>, ApiError> {
    info!(issue_id = %issue_id, "list_issue_versions invoked");
    let versions = state
        .get_issue_versions(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::issues::IssueVersionRecord::new(
                issue_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
            )
        })
        .collect();

    let response = v1::issues::ListIssueVersionsResponse::new(records);
    info!(
        issue_id = %issue_id,
        returned = response.versions.len(),
        "list_issue_versions completed"
    );
    Ok(Json(response))
}

pub async fn get_issue_version(
    State(state): State<AppState>,
    IssueVersionPath { issue_id, version }: IssueVersionPath,
) -> Result<Json<v1::issues::IssueVersionRecord>, ApiError> {
    info!(issue_id = %issue_id, version, "get_issue_version invoked");
    let versions = state
        .get_issue_versions(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!("issue '{issue_id}' version {version} not found"))
        })?;

    let response = v1::issues::IssueVersionRecord::new(
        issue_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
    );
    info!(issue_id = %issue_id, version, "get_issue_version completed");
    Ok(Json(response))
}

pub async fn list_issues(
    State(state): State<AppState>,
    Query(query): Query<v1::issues::SearchIssuesQuery>,
) -> Result<Json<v1::issues::ListIssuesResponse>, ApiError> {
    let query: SearchIssuesQuery = query.into();
    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        query = ?query.q,
        graph_filters = ?query.graph_filters,
        include_deleted = ?query.include_deleted,
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
    let include_deleted = query.include_deleted.unwrap_or(false);

    let issues = state
        .list_issues_with_deleted(include_deleted)
        .await
        .map_err(|err| map_issue_error(err, None))?;

    let issue_records: Vec<IssueRecord> = issues
        .into_iter()
        .map(|(id, issue)| IssueRecord::new(id, issue.item))
        .collect();

    let graph_matches = if query.graph_filters.is_empty() {
        None
    } else {
        Some(
            state
                .search_issue_graph(&query.graph_filters)
                .await
                .map_err(map_graph_filter_error)?,
        )
    };

    let filtered = issue_records
        .into_iter()
        .filter(|record| {
            issue_matches(
                query.issue_type,
                query.status,
                search_term.as_deref(),
                assignee_filter,
                &record.id,
                &record.issue,
            ) && graph_matches
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&record.id))
        })
        .collect();

    let response: v1::issues::ListIssuesResponse = ListIssuesResponse::new(filtered).into();
    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        returned = response.issues.len(),
        "list_issues completed"
    );
    Ok(Json(response))
}

pub async fn add_todo_item(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<v1::issues::AddTodoItemRequest>,
) -> Result<Json<v1::issues::TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "add_todo_item invoked");
    let request: AddTodoItemRequest = request.into();
    let todo_list = state
        .add_todo_item(
            issue_id.clone(),
            TodoItem::new(request.description, request.is_done),
        )
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "add_todo_item completed"
    );
    let response: v1::issues::TodoListResponse = TodoListResponse::new(issue_id, todo_list).into();
    Ok(Json(response))
}

pub async fn replace_todo_list(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<v1::issues::ReplaceTodoListRequest>,
) -> Result<Json<v1::issues::TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "replace_todo_list invoked");
    let request: ReplaceTodoListRequest = request.into();
    let todo_list = state
        .replace_todo_list(issue_id.clone(), request.todo_list)
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "replace_todo_list completed"
    );
    let response: v1::issues::TodoListResponse = TodoListResponse::new(issue_id, todo_list).into();
    Ok(Json(response))
}

pub async fn set_todo_item_status(
    State(state): State<AppState>,
    TodoItemPath {
        issue_id,
        item_number,
    }: TodoItemPath,
    Json(request): Json<v1::issues::SetTodoItemStatusRequest>,
) -> Result<Json<v1::issues::TodoListResponse>, ApiError> {
    info!(
        issue_id = %issue_id,
        item_number,
        desired_status = request.is_done,
        "set_todo_item_status invoked"
    );
    let request: SetTodoItemStatusRequest = request.into();
    let todo_list = state
        .set_todo_item_status(issue_id.clone(), item_number, request.is_done)
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        item_number,
        desired_status = request.is_done,
        "set_todo_item_status completed"
    );
    let response: v1::issues::TodoListResponse = TodoListResponse::new(issue_id, todo_list).into();
    Ok(Json(response))
}

fn map_graph_filter_error(err: StoreError) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => ApiError::bad_request(format!(
            "issue '{id}' referenced in graph filter does not exist"
        )),
        other => map_issue_error(other, None),
    }
}

fn map_upsert_issue_error(err: UpsertIssueError) -> ApiError {
    match err {
        UpsertIssueError::JobIdProvidedForUpdate => {
            ApiError::bad_request("job_id may only be provided when creating an issue")
        }
        UpsertIssueError::MissingCreator => ApiError::bad_request("issue creator must be set"),
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
    }
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
            || issue.progress.to_lowercase().contains(term)
            || issue.issue_type.as_str() == term
            || issue.status.as_str() == term
            || issue.creator.as_ref().to_lowercase().contains(term)
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

fn map_todo_error(err: UpdateTodoListError) -> ApiError {
    match err {
        UpdateTodoListError::IssueNotFound { issue_id, source } => {
            map_issue_error(source, Some(&issue_id))
        }
        UpdateTodoListError::InvalidItemNumber {
            issue_id,
            item_number,
        } => {
            error!(
                issue_id = %issue_id,
                item_number,
                "todo item number out of bounds"
            );
            ApiError::bad_request(format!(
                "todo item number {item_number} is out of range for issue '{issue_id}'"
            ))
        }
        UpdateTodoListError::Store { issue_id, source } => map_issue_error(source, Some(&issue_id)),
    }
}

pub async fn delete_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<v1::issues::IssueRecord>, ApiError> {
    info!(issue_id = %issue_id, "delete_issue invoked");
    state
        .delete_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let issue = state
        .get_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    info!(issue_id = %issue_id, "delete_issue completed");
    let response: v1::issues::IssueRecord = IssueRecord::new(issue_id, issue.item).into();
    Ok(Json(response))
}
