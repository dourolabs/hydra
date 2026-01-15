use crate::{
    app::{AppState, UpsertIssueError},
    routes::jobs::ApiError,
    routes::map_emit_error,
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::issues::{
    Issue, IssueDependencyType, IssueGraphFilter, IssueGraphFilterSide, IssueGraphWildcard,
    IssueId, IssueRecord, IssueStatus, IssueType, ListIssuesResponse, SearchIssuesQuery,
    UpsertIssueRequest, UpsertIssueResponse,
};
use std::collections::{HashMap, HashSet, VecDeque};
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
        let context = IssueGraphContext::new(&issue_records);
        Some(apply_graph_filters(&context, &query.graph_filters)?)
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

struct IssueGraphContext {
    known_issues: HashSet<IssueId>,
    forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
    reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
}

impl IssueGraphContext {
    fn new(records: &[IssueRecord]) -> Self {
        let mut known_issues = HashSet::new();
        let mut forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();

        for record in records {
            known_issues.insert(record.id.clone());
            for dependency in &record.issue.dependencies {
                let dependents = forward
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(dependency.issue_id.clone())
                    .or_default();
                dependents.push(record.id.clone());

                let targets = reverse
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(record.id.clone())
                    .or_default();
                targets.push(dependency.issue_id.clone());
            }
        }

        Self {
            known_issues,
            forward,
            reverse,
        }
    }

    fn contains_issue(&self, issue_id: &IssueId) -> bool {
        self.known_issues.contains(issue_id)
    }

    fn adjacency(
        &self,
        side: IssueGraphFilterSide,
        dependency_type: IssueDependencyType,
    ) -> Option<&HashMap<IssueId, Vec<IssueId>>> {
        match side {
            IssueGraphFilterSide::Left => self.forward.get(&dependency_type),
            IssueGraphFilterSide::Right => self.reverse.get(&dependency_type),
        }
    }
}

fn apply_graph_filters(
    context: &IssueGraphContext,
    filters: &[IssueGraphFilter],
) -> Result<HashSet<IssueId>, ApiError> {
    let mut intersection: Option<HashSet<IssueId>> = None;

    for filter in filters {
        let literal = filter.literal_issue_id();
        if !context.contains_issue(literal) {
            return Err(ApiError::bad_request(format!(
                "issue '{literal}' referenced in graph filter does not exist"
            )));
        }

        let adjacency = context.adjacency(filter.wildcard_position(), filter.dependency_type);

        let matches = collect_matches(adjacency, literal, filter.wildcard_kind());

        match &mut intersection {
            Some(existing) => existing.retain(|id| matches.contains(id)),
            None => intersection = Some(matches),
        }

        if let Some(existing) = &intersection {
            if existing.is_empty() {
                break;
            }
        }
    }

    Ok(intersection.unwrap_or_default())
}

fn collect_matches(
    adjacency: Option<&HashMap<IssueId, Vec<IssueId>>>,
    literal: &IssueId,
    wildcard: IssueGraphWildcard,
) -> HashSet<IssueId> {
    let Some(map) = adjacency else {
        return HashSet::new();
    };

    match wildcard {
        IssueGraphWildcard::Immediate => map
            .get(literal)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        IssueGraphWildcard::Transitive => {
            let mut matches = HashSet::new();
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();

            visited.insert(literal.clone());
            queue.push_back(literal.clone());

            while let Some(current) = queue.pop_front() {
                if let Some(neighbors) = map.get(&current) {
                    for neighbor in neighbors {
                        if visited.insert(neighbor.clone()) {
                            queue.push_back(neighbor.clone());
                        }
                        matches.insert(neighbor.clone());
                    }
                }
            }

            matches
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::issues::IssueDependency;
    use std::{collections::HashSet, str::FromStr};

    fn issue_id(value: &str) -> IssueId {
        IssueId::from_str(value).unwrap()
    }

    fn issue_record(id: &str, deps: Vec<(IssueDependencyType, &str)>) -> IssueRecord {
        IssueRecord {
            id: issue_id(id),
            issue: Issue {
                issue_type: IssueType::Task,
                description: format!("Issue {id}"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: deps
                    .into_iter()
                    .map(|(dependency_type, target)| IssueDependency {
                        dependency_type,
                        issue_id: issue_id(target),
                    })
                    .collect(),
                patches: Vec::new(),
            },
        }
    }

    fn filter(expr: &str) -> IssueGraphFilter {
        IssueGraphFilter::from_str(expr).unwrap()
    }

    #[test]
    fn graph_filter_returns_children() {
        let records = vec![
            issue_record("i-abcd", vec![]),
            issue_record("i-efgh", vec![(IssueDependencyType::ChildOf, "i-abcd")]),
            issue_record("i-ijkl", vec![(IssueDependencyType::ChildOf, "i-efgh")]),
        ];
        let context = IssueGraphContext::new(&records);
        let matches = apply_graph_filters(&context, &[filter("*:child-of:i-abcd")]).unwrap();
        assert_eq!(matches, HashSet::from([issue_id("i-efgh")]));
    }

    #[test]
    fn graph_filter_returns_transitive_children() {
        let records = vec![
            issue_record("i-abcd", vec![]),
            issue_record("i-efgh", vec![(IssueDependencyType::ChildOf, "i-abcd")]),
            issue_record("i-ijkl", vec![(IssueDependencyType::ChildOf, "i-efgh")]),
        ];
        let context = IssueGraphContext::new(&records);
        let matches = apply_graph_filters(&context, &[filter("**:child-of:i-abcd")]).unwrap();
        assert_eq!(
            matches,
            HashSet::from([issue_id("i-efgh"), issue_id("i-ijkl")])
        );
    }

    #[test]
    fn graph_filter_returns_ancestors_for_right_wildcards() {
        let records = vec![
            issue_record("i-abcd", vec![]),
            issue_record("i-efgh", vec![(IssueDependencyType::ChildOf, "i-abcd")]),
            issue_record("i-ijkl", vec![(IssueDependencyType::ChildOf, "i-efgh")]),
        ];
        let context = IssueGraphContext::new(&records);
        let matches = apply_graph_filters(&context, &[filter("i-ijkl:child-of:**")]).unwrap();
        assert_eq!(
            matches,
            HashSet::from([issue_id("i-abcd"), issue_id("i-efgh")])
        );
    }

    #[test]
    fn graph_filters_intersect_multiple_constraints() {
        let records = vec![
            issue_record("i-abcd", vec![]),
            issue_record("i-mnop", vec![]),
            issue_record(
                "i-qrst",
                vec![
                    (IssueDependencyType::ChildOf, "i-abcd"),
                    (IssueDependencyType::BlockedOn, "i-mnop"),
                ],
            ),
            issue_record("i-uvwx", vec![(IssueDependencyType::ChildOf, "i-abcd")]),
            issue_record("i-yzab", vec![(IssueDependencyType::BlockedOn, "i-mnop")]),
        ];
        let context = IssueGraphContext::new(&records);
        let matches = apply_graph_filters(
            &context,
            &[filter("*:child-of:i-abcd"), filter("*:blocked-on:i-mnop")],
        )
        .unwrap();
        assert_eq!(matches, HashSet::from([issue_id("i-qrst")]));
    }

    #[test]
    fn graph_filter_errors_when_literal_missing() {
        let records = vec![issue_record("i-abcd", vec![])];
        let context = IssueGraphContext::new(&records);
        let result = apply_graph_filters(&context, &[filter("*:child-of:i-cdef")]);
        assert!(result.is_err());
    }
}
