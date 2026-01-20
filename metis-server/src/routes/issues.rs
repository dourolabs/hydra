use crate::{
    app::{AppState, UpdateTodoListError, UpsertIssueError},
    routes::jobs::ApiError,
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::issues::{
    AddTodoItemRequest, Issue, IssueId, IssueRecord, IssueStatus, IssueType, ListIssuesResponse,
    ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest, TodoItem,
    TodoListResponse, UpsertIssueRequest, UpsertIssueResponse,
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
    Json(payload): Json<UpsertIssueRequest>,
) -> Result<Json<UpsertIssueResponse>, ApiError> {
    info!("create_issue invoked");
    let issue_id = state
        .upsert_issue(None, payload)
        .await
        .map_err(map_upsert_issue_error)?;

    info!(issue_id = %issue_id, "create_issue completed");
    Ok(Json(UpsertIssueResponse { issue_id }))
}

pub async fn update_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(payload): Json<UpsertIssueRequest>,
) -> Result<Json<UpsertIssueResponse>, ApiError> {
    info!(issue_id = %issue_id, "update_issue invoked");
    let issue_id = state
        .upsert_issue(Some(issue_id), payload)
        .await
        .map_err(map_upsert_issue_error)?;

    info!(issue_id = %issue_id, "update_issue completed");
    Ok(Json(UpsertIssueResponse { issue_id }))
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

    info!(issue_id = %issue_id, "get_issue completed");
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
        graph_filters = ?query.graph_filters,
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

    let issue_records: Vec<IssueRecord> = issues
        .into_iter()
        .map(|(id, issue)| IssueRecord { id, issue })
        .collect();

    let graph_matches = if query.graph_filters.is_empty() {
        None
    } else {
        Some(
            store_read
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

    let response = ListIssuesResponse { issues: filtered };
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
    Json(request): Json<AddTodoItemRequest>,
) -> Result<Json<TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "add_todo_item invoked");
    let todo_list = state
        .add_todo_item(
            issue_id.clone(),
            TodoItem {
                description: request.description,
                is_done: request.is_done,
            },
        )
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "add_todo_item completed"
    );
    Ok(Json(TodoListResponse {
        issue_id,
        todo_list,
    }))
}

pub async fn replace_todo_list(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<ReplaceTodoListRequest>,
) -> Result<Json<TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "replace_todo_list invoked");
    let todo_list = state
        .replace_todo_list(issue_id.clone(), request.todo_list)
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "replace_todo_list completed"
    );
    Ok(Json(TodoListResponse {
        issue_id,
        todo_list,
    }))
}

pub async fn set_todo_item_status(
    State(state): State<AppState>,
    TodoItemPath {
        issue_id,
        item_number,
    }: TodoItemPath,
    Json(request): Json<SetTodoItemStatusRequest>,
) -> Result<Json<TodoListResponse>, ApiError> {
    info!(
        issue_id = %issue_id,
        item_number,
        desired_status = request.is_done,
        "set_todo_item_status invoked"
    );
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
    Ok(Json(TodoListResponse {
        issue_id,
        todo_list,
    }))
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
            || issue.creator.to_lowercase().contains(term)
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
