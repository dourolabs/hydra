use crate::{
    app::{AppState, UpsertIssueError},
    routes::jobs::ApiError,
    routes::jobs::job_record_with_time,
    routes::map_emit_error,
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::{DateTime, Utc};
use metis_common::issues::{
    Issue, IssueDependencyType, IssueDetailsQuery, IssueDetailsResponse, IssueId, IssueRecord,
    IssueStatus, IssueStatusDetails, IssueTaskDetail, IssueTreeNode, IssueType, ListIssuesResponse,
    SearchIssuesQuery, UpsertIssueRequest, UpsertIssueResponse,
};
use std::collections::{HashMap, HashSet};
use tracing::{error, info};

const DEFAULT_ISSUE_LOG_TAIL_LINES: i64 = 200;

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
    let issue_id = state
        .upsert_issue(None, payload)
        .await
        .map_err(map_upsert_issue_error)?;

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

    Ok(Json(IssueRecord {
        id: issue_id,
        issue,
    }))
}

pub async fn get_issue_details(
    State(state): State<AppState>,
    IssueIdPath(issue_id): IssueIdPath,
    Query(query): Query<IssueDetailsQuery>,
) -> Result<Json<IssueDetailsResponse>, ApiError> {
    info!(issue_id = %issue_id, "get_issue_details invoked");
    let tail_lines = query.tail_lines.or(Some(DEFAULT_ISSUE_LOG_TAIL_LINES));

    let store_read = state.store.read().await;
    let store = store_read.as_ref();
    let issue = store
        .get_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;
    let issue_record = IssueRecord {
        id: issue_id.clone(),
        issue: issue.clone(),
    };

    let mut issue_map: HashMap<IssueId, Issue> = store
        .list_issues()
        .await
        .map_err(|err| map_issue_error(err, None))?
        .into_iter()
        .collect();
    issue_map
        .entry(issue_id.clone())
        .or_insert_with(|| issue.clone());

    let children_map = build_children_map(&issue_map);
    let is_ready = store
        .is_issue_ready(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;
    let blockers = issue_blockers(&issue, &issue_map);
    let open_children = issue_open_children(&issue_id, &issue_map, &children_map);
    let status = IssueStatusDetails {
        status: issue.status,
        is_ready,
        blockers,
        open_children,
    };

    let subtask_tree = build_issue_tree_root(&issue_id, &issue, &issue_map, &children_map);

    let task_ids = store
        .get_tasks_for_issue(&issue_id)
        .await
        .map_err(|err| map_issue_error(err, Some(&issue_id)))?;
    let mut task_records: Vec<(IssueTaskDetail, Option<DateTime<Utc>>)> = Vec::new();
    for task_id in task_ids {
        let (job, reference_time) = job_record_with_time(&task_id, store).await.map_err(|err| {
            error!(job_id = %task_id, error = %err, "failed to load task details");
            ApiError::internal(anyhow!(
                "failed to load task '{task_id}' for issue details: {err}"
            ))
        })?;

        task_records.push((
            IssueTaskDetail {
                job,
                logs: None,
                log_error: None,
            },
            reference_time,
        ));
    }
    drop(store_read);

    task_records.sort_by(|a, b| b.1.cmp(&a.1));
    let mut tasks = Vec::with_capacity(task_records.len());
    for (mut detail, _) in task_records {
        match state.job_engine.get_logs(&detail.job.id, tail_lines).await {
            Ok(logs) => detail.logs = Some(logs),
            Err(err) => {
                error!(
                    job_id = %detail.job.id,
                    error = ?err,
                    "failed to fetch logs for issue details"
                );
                detail.log_error = Some(err.to_string());
            }
        }
        tasks.push(detail);
    }

    Ok(Json(IssueDetailsResponse {
        issue: issue_record,
        status,
        subtasks: subtask_tree,
        tasks,
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

    Ok(Json(ListIssuesResponse { issues: filtered }))
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
            ApiError::bad_request("job_id must reference a running job to record emitted artifacts")
        }
        UpsertIssueError::EmitArtifacts { job_id, source } => {
            map_emit_error(source, job_id.as_ref())
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
            || issue
                .assignee
                .as_deref()
                .map(|value| value.to_lowercase().contains(term))
                .unwrap_or(false);
    }

    true
}

fn build_children_map(issues: &HashMap<IssueId, Issue>) -> HashMap<IssueId, Vec<IssueId>> {
    let mut children_map: HashMap<IssueId, Vec<IssueId>> = HashMap::new();
    for (issue_id, issue) in issues {
        for dependency in &issue.dependencies {
            if dependency.dependency_type == IssueDependencyType::ChildOf {
                let children = children_map.entry(dependency.issue_id.clone()).or_default();
                if !children.contains(issue_id) {
                    children.push(issue_id.clone());
                }
            }
        }
    }

    for children in children_map.values_mut() {
        children.sort();
    }

    children_map
}

fn build_issue_tree_root(
    issue_id: &IssueId,
    issue: &Issue,
    issues: &HashMap<IssueId, Issue>,
    children_map: &HashMap<IssueId, Vec<IssueId>>,
) -> IssueTreeNode {
    let mut visited = HashSet::new();
    build_issue_tree(issue_id, issue.clone(), issues, children_map, &mut visited)
}

fn build_issue_tree(
    issue_id: &IssueId,
    issue: Issue,
    issues: &HashMap<IssueId, Issue>,
    children_map: &HashMap<IssueId, Vec<IssueId>>,
    visited: &mut HashSet<IssueId>,
) -> IssueTreeNode {
    let mut node = IssueTreeNode {
        issue: IssueRecord {
            id: issue_id.clone(),
            issue,
        },
        children: Vec::new(),
    };

    if !visited.insert(issue_id.clone()) {
        return node;
    }

    let Some(children) = children_map.get(issue_id) else {
        return node;
    };

    for child_id in children {
        if visited.contains(child_id) {
            continue;
        }
        let Some(child_issue) = issues.get(child_id).cloned() else {
            continue;
        };
        node.children.push(build_issue_tree(
            child_id,
            child_issue,
            issues,
            children_map,
            visited,
        ));
    }

    node
}

fn issue_blockers(issue: &Issue, issues: &HashMap<IssueId, Issue>) -> Vec<IssueId> {
    let mut blockers: Vec<IssueId> = issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
        .filter(|dependency| {
            issues
                .get(&dependency.issue_id)
                .map(|blocked| blocked.status != IssueStatus::Closed)
                .unwrap_or(true)
        })
        .map(|dependency| dependency.issue_id.clone())
        .collect();
    blockers.sort();
    blockers
}

fn issue_open_children(
    issue_id: &IssueId,
    issues: &HashMap<IssueId, Issue>,
    children_map: &HashMap<IssueId, Vec<IssueId>>,
) -> Vec<IssueId> {
    let mut open_children = children_map
        .get(issue_id)
        .map(|children| {
            children
                .iter()
                .filter(|child_id| {
                    issues
                        .get(*child_id)
                        .map(|child| child.status != IssueStatus::Closed)
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<IssueId>>()
        })
        .unwrap_or_default();
    open_children.sort();
    open_children
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
