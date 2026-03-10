use crate::domain::actors::{Actor, ActorRef};
use crate::domain::issues::SubtreeIssueRow;
use crate::domain::issues::TodoItem;
use crate::{
    app::{AppState, UpdateTodoListError, UpsertIssueError},
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{
    IssueId, MetisId,
    api::v1::{
        ApiError, issues as api_issues,
        pagination::{compute_next_cursor, effective_limit},
    },
};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{error, info};

#[derive(Debug, Deserialize)]
pub struct GetIssueQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
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
    Extension(actor): Extension<Actor>,
    Json(payload): Json<api_issues::UpsertIssueRequest>,
) -> Result<Json<api_issues::UpsertIssueResponse>, ApiError> {
    info!("create_issue invoked");
    let (issue_id, version) = state
        .upsert_issue(None, payload, ActorRef::from(&actor))
        .await
        .map_err(map_upsert_issue_error)?;

    info!(issue_id = %issue_id, "create_issue completed");
    Ok(Json(api_issues::UpsertIssueResponse::new(
        issue_id, version,
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

    info!(issue_id = %issue_id, "update_issue completed");
    Ok(Json(api_issues::UpsertIssueResponse::new(
        issue_id, version,
    )))
}

pub async fn get_issue(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Query(query): Query<GetIssueQuery>,
) -> Result<Json<api_issues::IssueVersionRecord>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    info!(issue_id = %issue_id, include_deleted, "get_issue invoked");
    let issue = state
        .get_issue(&issue_id, include_deleted)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let object_id = MetisId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(issue_id = %issue_id, "get_issue completed");
    let response = api_issues::IssueVersionRecord::new(
        issue_id,
        issue.version,
        issue.timestamp,
        issue.item.into(),
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

    let object_id = MetisId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let records = versions
        .into_iter()
        .map(|version| {
            api_issues::IssueVersionRecord::new(
                issue_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
                version.actor,
                version.creation_time,
                labels.clone(),
            )
        })
        .collect();

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

    let object_id = MetisId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let response = api_issues::IssueVersionRecord::new(
        issue_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
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
    let graph_filters: Vec<_> = query
        .graph_filters
        .iter()
        .cloned()
        .map(Into::into)
        .collect();

    info!(
        issue_type = ?query.issue_type,
        status = ?query.status,
        assignee = ?query.assignee,
        query = ?query.q,
        graph_filters = ?graph_filters,
        include_deleted = ?query.include_deleted,
        label_ids = ?query.label_ids,
        "list_issues invoked"
    );

    // Pass the query to the store for filtering (except graph filters)
    let issues = state
        .list_issues_with_query(&query)
        .await
        .map_err(|err| map_issue_error(err, None))?;

    // Graph filtering stays in routes layer as it requires graph traversal
    let graph_matches = if graph_filters.is_empty() {
        None
    } else {
        Some(
            state
                .search_issue_graph(&graph_filters)
                .await
                .map_err(map_graph_filter_error)?,
        )
    };

    // Apply graph filter first to reduce the set before batch label lookup
    let issues: Vec<_> = if let Some(ref allowed) = graph_matches {
        issues
            .into_iter()
            .filter(|(id, _)| allowed.contains(id))
            .collect()
    } else {
        issues
    };

    // Batch-fetch labels for all issues in a single query
    let object_ids: Vec<MetisId> = issues
        .iter()
        .map(|(id, _)| MetisId::from(id.clone()))
        .collect();
    let labels_map = state
        .get_labels_for_objects(&object_ids)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to batch-fetch labels for issues");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let eff_limit = effective_limit(query.limit);

    // Optionally compute jobs summary per issue
    let wants_jobs_summary = query.include_job_status.unwrap_or(false);
    let jobs_summary_map: HashMap<IssueId, api_issues::JobStatusSummary> = if wants_jobs_summary {
        build_jobs_summary_map(&state, &issues).await?
    } else {
        HashMap::new()
    };

    let mut filtered: Vec<api_issues::IssueSummaryRecord> = Vec::new();
    for (id, versioned) in issues {
        let object_id = MetisId::from(id.clone());
        let labels = labels_map.get(&object_id).cloned().unwrap_or_default();

        let api_issue: api_issues::Issue = versioned.item.into();
        let summary = api_issues::IssueSummary::from(&api_issue);
        let mut record = api_issues::IssueSummaryRecord::new(
            id.clone(),
            versioned.version,
            versioned.timestamp,
            summary,
            versioned.actor,
            versioned.creation_time,
            labels,
        );
        if wants_jobs_summary {
            record.jobs_summary = jobs_summary_map.get(&id).cloned();
        }
        filtered.push(record);
    }

    let next_cursor = compute_next_cursor(
        &mut filtered,
        eff_limit,
        |r| &r.timestamp,
        |r| r.issue_id.as_ref(),
    );

    // If include_subtree is set, fetch and attach subtrees
    if query.include_subtree {
        let root_ids: Vec<_> = filtered.iter().map(|r| r.issue_id.clone()).collect();
        let subtree_rows = state.get_issue_subtrees(&root_ids).await.map_err(|err| {
            error!(error = %err, "failed to fetch issue subtrees");
            ApiError::internal(anyhow!("failed to fetch issue subtrees: {err}"))
        })?;
        let subtree_map = assemble_subtrees(&root_ids, subtree_rows);
        for record in &mut filtered {
            record.subtree = Some(
                subtree_map
                    .get(&record.issue_id)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }

    // Compute total_count when count=true
    let total_count = if query.count == Some(true) {
        if graph_matches.is_some() {
            // When graph filters are active, we need to fetch all matching issues
            // (without pagination) and apply the graph filter to get the correct count.
            let mut count_query = query.clone();
            count_query.limit = None;
            count_query.cursor = None;
            let all_issues = state
                .list_issues_with_query(&count_query)
                .await
                .map_err(|err| map_issue_error(err, None))?;
            let graph_allowed = graph_matches.as_ref().unwrap();
            let count = all_issues
                .iter()
                .filter(|(id, _)| graph_allowed.contains(id))
                .count() as u64;
            Some(count)
        } else {
            let count = state
                .count_issues(&query)
                .await
                .map_err(|err| map_issue_error(err, None))?;
            Some(count)
        }
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

pub async fn add_todo_item(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<api_issues::AddTodoItemRequest>,
) -> Result<Json<api_issues::TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "add_todo_item invoked");
    let todo_list = state
        .add_todo_item(
            issue_id.clone(),
            TodoItem::new(request.description, request.is_done),
            ActorRef::from(&actor),
        )
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "add_todo_item completed"
    );
    let response = api_issues::TodoListResponse::new(
        issue_id,
        todo_list.into_iter().map(Into::into).collect(),
    );
    Ok(Json(response))
}

pub async fn replace_todo_list(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<api_issues::ReplaceTodoListRequest>,
) -> Result<Json<api_issues::TodoListResponse>, ApiError> {
    info!(issue_id = %issue_id, "replace_todo_list invoked");
    let todo_list = state
        .replace_todo_list(
            issue_id.clone(),
            request.todo_list.into_iter().map(Into::into).collect(),
            ActorRef::from(&actor),
        )
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        count = todo_list.len(),
        "replace_todo_list completed"
    );
    let response = api_issues::TodoListResponse::new(
        issue_id,
        todo_list.into_iter().map(Into::into).collect(),
    );
    Ok(Json(response))
}

pub async fn set_todo_item_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    TodoItemPath {
        issue_id,
        item_number,
    }: TodoItemPath,
    Json(request): Json<api_issues::SetTodoItemStatusRequest>,
) -> Result<Json<api_issues::TodoListResponse>, ApiError> {
    info!(
        issue_id = %issue_id,
        item_number,
        desired_status = request.is_done,
        "set_todo_item_status invoked"
    );
    let todo_list = state
        .set_todo_item_status(
            issue_id.clone(),
            item_number,
            request.is_done,
            ActorRef::from(&actor),
        )
        .await
        .map_err(map_todo_error)?;

    info!(
        issue_id = %issue_id,
        item_number,
        desired_status = request.is_done,
        "set_todo_item_status completed"
    );
    let response = api_issues::TodoListResponse::new(
        issue_id,
        todo_list.into_iter().map(Into::into).collect(),
    );
    Ok(Json(response))
}

/// Build a map of IssueId → JobStatusSummary using store-level aggregation.
async fn build_jobs_summary_map(
    state: &AppState,
    issues: &[(
        IssueId,
        metis_common::Versioned<crate::domain::issues::Issue>,
    )],
) -> Result<HashMap<IssueId, api_issues::JobStatusSummary>, ApiError> {
    let issue_ids: Vec<IssueId> = issues.iter().map(|(id, _)| id.clone()).collect();
    if issue_ids.is_empty() {
        return Ok(HashMap::new());
    }

    state
        .get_jobs_summary_for_issues(&issue_ids)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to fetch jobs summary");
            ApiError::internal(anyhow!("failed to fetch jobs summary: {err}"))
        })
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
        UpsertIssueError::PolicyViolation(violation) => ApiError::bad_request(violation.message),
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
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
) -> Result<Json<api_issues::IssueVersionRecord>, ApiError> {
    info!(issue_id = %issue_id, "delete_issue invoked");
    state
        .delete_issue(&issue_id, ActorRef::from(&actor))
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let issue = state
        .get_issue(&issue_id, true)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;

    let object_id = MetisId::from(issue_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(issue_id = %issue_id, error = %err, "failed to fetch labels for issue");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(issue_id = %issue_id, "delete_issue completed");
    let response = api_issues::IssueVersionRecord::new(
        issue_id,
        issue.version,
        issue.timestamp,
        issue.item.into(),
        issue.actor,
        issue.creation_time,
        labels,
    );
    Ok(Json(response))
}

/// Assembles flat subtree rows into nested tree structures keyed by root issue ID.
fn assemble_subtrees(
    root_ids: &[IssueId],
    rows: Vec<SubtreeIssueRow>,
) -> HashMap<IssueId, Vec<api_issues::SubtreeIssue>> {
    // Build a map from parent_id -> list of children rows
    let mut children_map: HashMap<IssueId, Vec<SubtreeIssueRow>> = HashMap::new();
    for row in rows {
        children_map
            .entry(row.parent_id.clone())
            .or_default()
            .push(row);
    }

    // Recursively build nested SubtreeIssue nodes
    fn build_children(
        parent_id: &IssueId,
        children_map: &HashMap<IssueId, Vec<SubtreeIssueRow>>,
    ) -> Vec<api_issues::SubtreeIssue> {
        let Some(children) = children_map.get(parent_id) else {
            return Vec::new();
        };
        children
            .iter()
            .map(|row| {
                api_issues::SubtreeIssue::new(
                    row.issue_id.clone(),
                    row.status.into(),
                    row.has_active_task,
                    row.assignee.clone(),
                    row.title.clone(),
                    build_children(&row.issue_id, children_map),
                )
            })
            .collect()
    }

    let mut result = HashMap::new();
    for root_id in root_ids {
        result.insert(root_id.clone(), build_children(root_id, &children_map));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::IssueStatus;

    fn row(issue: &IssueId, parent: &IssueId, title: &str) -> SubtreeIssueRow {
        SubtreeIssueRow {
            issue_id: issue.clone(),
            parent_id: parent.clone(),
            status: IssueStatus::Open,
            title: title.to_string(),
            assignee: None,
            has_active_task: false,
        }
    }

    #[test]
    fn assemble_subtrees_empty_rows() {
        let root = IssueId::new();
        let result = assemble_subtrees(&[root.clone()], vec![]);
        assert_eq!(result[&root].len(), 0);
    }

    #[test]
    fn assemble_subtrees_single_child() {
        let root = IssueId::new();
        let child = IssueId::new();
        let rows = vec![row(&child, &root, "child")];
        let result = assemble_subtrees(&[root.clone()], rows);
        assert_eq!(result[&root].len(), 1);
        assert_eq!(result[&root][0].issue_id, child);
        assert!(result[&root][0].children.is_empty());
    }

    #[test]
    fn assemble_subtrees_multiple_levels() {
        let root = IssueId::new();
        let child = IssueId::new();
        let grandchild = IssueId::new();
        let rows = vec![
            row(&child, &root, "child"),
            row(&grandchild, &child, "grandchild"),
        ];
        let result = assemble_subtrees(&[root.clone()], rows);
        assert_eq!(result[&root].len(), 1);
        assert_eq!(result[&root][0].children.len(), 1);
        assert_eq!(result[&root][0].children[0].issue_id, grandchild);
    }

    #[test]
    fn assemble_subtrees_multiple_roots() {
        let root_a = IssueId::new();
        let root_b = IssueId::new();
        let child_a = IssueId::new();
        let child_b = IssueId::new();
        let rows = vec![
            row(&child_a, &root_a, "child_a"),
            row(&child_b, &root_b, "child_b"),
        ];
        let result = assemble_subtrees(&[root_a.clone(), root_b.clone()], rows);
        assert_eq!(result[&root_a].len(), 1);
        assert_eq!(result[&root_a][0].issue_id, child_a);
        assert_eq!(result[&root_b].len(), 1);
        assert_eq!(result[&root_b][0].issue_id, child_b);
    }

    #[test]
    fn assemble_subtrees_preserves_has_active_task() {
        let root = IssueId::new();
        let child = IssueId::new();
        let rows = vec![SubtreeIssueRow {
            issue_id: child.clone(),
            parent_id: root.clone(),
            status: IssueStatus::InProgress,
            title: "active child".to_string(),
            assignee: Some("alice".to_string()),
            has_active_task: true,
        }];
        let result = assemble_subtrees(&[root.clone()], rows);
        assert!(result[&root][0].has_active_task);
        assert_eq!(result[&root][0].assignee, Some("alice".to_string()));
        assert_eq!(result[&root][0].status, api_issues::IssueStatus::InProgress);
    }

    #[test]
    fn assemble_subtrees_root_with_no_children() {
        let root_with_children = IssueId::new();
        let root_without = IssueId::new();
        let child = IssueId::new();
        let rows = vec![row(&child, &root_with_children, "child")];
        let result = assemble_subtrees(&[root_with_children.clone(), root_without.clone()], rows);
        assert_eq!(result[&root_with_children].len(), 1);
        assert_eq!(result[&root_without].len(), 0);
    }
}
