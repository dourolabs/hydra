pub use crate::IssueId;
use crate::{PatchId, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IssueStatus {
    #[default]
    Open,
    InProgress,
    Closed,
    Dropped,
}

impl IssueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueStatus::Open => "open",
            IssueStatus::InProgress => "in-progress",
            IssueStatus::Closed => "closed",
            IssueStatus::Dropped => "dropped",
        }
    }
}

impl fmt::Display for IssueStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IssueStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "open" => Ok(IssueStatus::Open),
            "in-progress" | "inprogress" | "in_progress" => Ok(IssueStatus::InProgress),
            "closed" => Ok(IssueStatus::Closed),
            "dropped" => Ok(IssueStatus::Dropped),
            other => Err(format!("unsupported issue status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueType {
    Bug,
    Feature,
    Task,
    Chore,
    #[serde(rename = "merge-request")]
    MergeRequest,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::Bug => "bug",
            IssueType::Feature => "feature",
            IssueType::Task => "task",
            IssueType::Chore => "chore",
            IssueType::MergeRequest => "merge-request",
        }
    }
}

impl fmt::Display for IssueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IssueType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "bug" => Ok(IssueType::Bug),
            "feature" => Ok(IssueType::Feature),
            "task" => Ok(IssueType::Task),
            "chore" => Ok(IssueType::Chore),
            "merge-request" | "mergerequest" | "merge_request" => Ok(IssueType::MergeRequest),
            other => Err(format!("unsupported issue type '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IssueDependencyType {
    ChildOf,
    BlockedOn,
}

impl IssueDependencyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueDependencyType::ChildOf => "child-of",
            IssueDependencyType::BlockedOn => "blocked-on",
        }
    }
}

impl fmt::Display for IssueDependencyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IssueDependencyType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "child-of" | "childof" | "child_of" => Ok(IssueDependencyType::ChildOf),
            "blocked-on" | "blockedon" | "blocked_on" => Ok(IssueDependencyType::BlockedOn),
            other => Err(format!("unsupported issue dependency type '{other}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IssueDependency {
    #[serde(rename = "type")]
    pub dependency_type: IssueDependencyType,
    pub issue_id: IssueId,
}

impl IssueDependency {
    pub fn new(dependency_type: IssueDependencyType, issue_id: IssueId) -> Self {
        Self {
            dependency_type,
            issue_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TodoItem {
    pub description: String,
    #[serde(default)]
    pub is_done: bool,
}

impl TodoItem {
    pub fn new(description: String, is_done: bool) -> Self {
        Self {
            description,
            is_done,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TodoListResponse {
    pub issue_id: IssueId,
    #[serde(default)]
    pub todo_list: Vec<TodoItem>,
}

impl TodoListResponse {
    pub fn new(issue_id: IssueId, todo_list: Vec<TodoItem>) -> Self {
        Self {
            issue_id,
            todo_list,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AddTodoItemRequest {
    pub description: String,
    #[serde(default)]
    pub is_done: bool,
}

impl AddTodoItemRequest {
    pub fn new(description: String, is_done: bool) -> Self {
        Self {
            description,
            is_done,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReplaceTodoListRequest {
    #[serde(default)]
    pub todo_list: Vec<TodoItem>,
}

impl ReplaceTodoListRequest {
    pub fn new(todo_list: Vec<TodoItem>) -> Self {
        Self { todo_list }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SetTodoItemStatusRequest {
    pub is_done: bool,
}

impl SetTodoItemStatusRequest {
    pub fn new(is_done: bool) -> Self {
        Self { is_done }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueGraphFilterSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueGraphWildcard {
    Immediate,
    Transitive,
}

impl IssueGraphWildcard {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueGraphWildcard::Immediate => "*",
            IssueGraphWildcard::Transitive => "**",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueGraphSelector {
    Issue(IssueId),
    Wildcard(IssueGraphWildcard),
}

impl IssueGraphSelector {
    fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("graph selector must not be empty".to_string());
        }

        match trimmed {
            "*" => Ok(IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate)),
            "**" => Ok(IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive)),
            _ => trimmed
                .parse::<IssueId>()
                .map(IssueGraphSelector::Issue)
                .map_err(|err| err.to_string()),
        }
    }

    fn as_str(&self) -> String {
        match self {
            IssueGraphSelector::Issue(id) => id.to_string(),
            IssueGraphSelector::Wildcard(wildcard) => wildcard.as_str().to_string(),
        }
    }
}

impl fmt::Display for IssueGraphSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssueGraphFilter {
    pub lhs: IssueGraphSelector,
    pub dependency_type: IssueDependencyType,
    pub rhs: IssueGraphSelector,
}

impl IssueGraphFilter {
    pub fn new(
        lhs: IssueGraphSelector,
        dependency_type: IssueDependencyType,
        rhs: IssueGraphSelector,
    ) -> Result<Self, String> {
        let lhs_is_wildcard = matches!(lhs, IssueGraphSelector::Wildcard(_));
        let rhs_is_wildcard = matches!(rhs, IssueGraphSelector::Wildcard(_));

        if lhs_is_wildcard == rhs_is_wildcard {
            return Err(
                "graph filters must have exactly one wildcard (*) or (**) selector".to_string(),
            );
        }

        Ok(Self {
            lhs,
            dependency_type,
            rhs,
        })
    }

    pub fn wildcard_position(&self) -> IssueGraphFilterSide {
        if matches!(self.lhs, IssueGraphSelector::Wildcard(_)) {
            IssueGraphFilterSide::Left
        } else {
            IssueGraphFilterSide::Right
        }
    }

    pub fn wildcard_kind(&self) -> IssueGraphWildcard {
        match (&self.lhs, &self.rhs) {
            (IssueGraphSelector::Wildcard(kind), _) => *kind,
            (_, IssueGraphSelector::Wildcard(kind)) => *kind,
            _ => unreachable!("graph filters always have a wildcard selector"),
        }
    }

    pub fn literal_issue_id(&self) -> &IssueId {
        match (&self.lhs, &self.rhs) {
            (IssueGraphSelector::Issue(id), IssueGraphSelector::Wildcard(_)) => id,
            (IssueGraphSelector::Wildcard(_), IssueGraphSelector::Issue(id)) => id,
            _ => unreachable!("graph filters always have exactly one literal selector"),
        }
    }
}

impl fmt::Display for IssueGraphFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.lhs, self.dependency_type, self.rhs)
    }
}

impl FromStr for IssueGraphFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, ':');
        let lhs = parts
            .next()
            .ok_or_else(|| "graph filter must include a left selector".to_string())?;
        let dependency_type = parts
            .next()
            .ok_or_else(|| "graph filter must include a dependency type".to_string())?;
        let rhs = parts
            .next()
            .ok_or_else(|| "graph filter must include a right selector".to_string())?;

        let lhs = IssueGraphSelector::parse(lhs)?;
        let rhs = IssueGraphSelector::parse(rhs)?;
        let dependency_type = IssueDependencyType::from_str(dependency_type)
            .map_err(|err| format!("invalid dependency type in graph filter: {err}"))?;
        IssueGraphFilter::new(lhs, dependency_type, rhs)
    }
}

impl Serialize for IssueGraphFilter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IssueGraphFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Issue {
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    pub description: String,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub status: IssueStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo_list: Vec<TodoItem>,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
}

impl Issue {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issue_type: IssueType,
        description: String,
        creator: String,
        progress: String,
        status: IssueStatus,
        assignee: Option<String>,
        todo_list: Vec<TodoItem>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
    ) -> Self {
        Self {
            issue_type,
            description,
            creator,
            progress,
            status,
            assignee,
            todo_list,
            dependencies,
            patches,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IssueRecord {
    pub id: IssueId,
    pub issue: Issue,
}

impl IssueRecord {
    pub fn new(id: IssueId, issue: Issue) -> Self {
        Self { id, issue }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertIssueRequest {
    pub issue: Issue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<TaskId>,
}

impl UpsertIssueRequest {
    pub fn new(issue: Issue, job_id: Option<TaskId>) -> Self {
        Self { issue, job_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertIssueResponse {
    pub issue_id: IssueId,
}

impl UpsertIssueResponse {
    pub fn new(issue_id: IssueId) -> Self {
        Self { issue_id }
    }
}

fn serialize_graph_filters<S>(
    filters: &[IssueGraphFilter],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = filters
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(",");
    serializer.serialize_str(&s)
}

fn deserialize_graph_filters<'de, D>(deserializer: D) -> Result<Vec<IssueGraphFilter>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|part| part.parse().map_err(de::Error::custom))
        .collect()
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchIssuesQuery {
    #[serde(default)]
    pub issue_type: Option<IssueType>,
    #[serde(default)]
    pub status: Option<IssueStatus>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(
        default,
        rename = "graph",
        serialize_with = "serialize_graph_filters",
        deserialize_with = "deserialize_graph_filters"
    )]
    pub graph_filters: Vec<IssueGraphFilter>,
}

impl SearchIssuesQuery {
    pub fn new(
        issue_type: Option<IssueType>,
        status: Option<IssueStatus>,
        assignee: Option<String>,
        q: Option<String>,
        graph_filters: Vec<IssueGraphFilter>,
    ) -> Self {
        Self {
            issue_type,
            status,
            assignee,
            q,
            graph_filters,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListIssuesResponse {
    pub issues: Vec<IssueRecord>,
}

impl ListIssuesResponse {
    pub fn new(issues: Vec<IssueRecord>) -> Self {
        Self { issues }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use serde_json::json;
    use std::collections::HashMap;

    fn issue_id(value: &str) -> IssueId {
        value.parse().unwrap()
    }

    #[test]
    fn parse_graph_filter_with_children() {
        let filter: IssueGraphFilter = "*:child-of:i-abcd".parse().unwrap();
        assert!(matches!(
            filter.lhs,
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate)
        ));
        assert_eq!(filter.dependency_type, IssueDependencyType::ChildOf);
        assert_eq!(filter.wildcard_position(), IssueGraphFilterSide::Left);
        let expected = issue_id("i-abcd");
        assert_eq!(filter.literal_issue_id(), &expected);
    }

    #[test]
    fn parse_graph_filter_with_blockers() {
        let filter: IssueGraphFilter = "i-efgh:blocked-on:**".parse().unwrap();
        assert!(matches!(
            filter.rhs,
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive)
        ));
        assert_eq!(filter.dependency_type, IssueDependencyType::BlockedOn);
        assert_eq!(filter.wildcard_position(), IssueGraphFilterSide::Right);
        assert_eq!(filter.wildcard_kind(), IssueGraphWildcard::Transitive);
        let expected = issue_id("i-efgh");
        assert_eq!(filter.literal_issue_id(), &expected);
    }

    #[test]
    fn graph_filter_rejects_missing_literal() {
        assert!("**:child-of:*".parse::<IssueGraphFilter>().is_err());
    }

    #[test]
    fn graph_filter_formats_to_string() {
        let filter: IssueGraphFilter = "*:child-of:i-qrst".parse().unwrap();
        assert_eq!(filter.to_string(), "*:child-of:i-qrst");
    }

    #[test]
    fn search_issues_query_serializes_with_reqwest() {
        let query = SearchIssuesQuery {
            issue_type: Some(IssueType::Bug),
            status: Some(IssueStatus::Open),
            assignee: Some("alice".to_string()),
            q: Some("test query".to_string()),
            graph_filters: vec![],
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("issue_type").map(String::as_str), Some("bug"));
        assert_eq!(params.get("status").map(String::as_str), Some("open"));
        assert_eq!(params.get("assignee").map(String::as_str), Some("alice"));
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
    }

    #[test]
    fn search_issues_query_serializes_with_graph_filters() {
        let filter1: IssueGraphFilter = "*:child-of:i-abcd".parse().unwrap();
        let filter2: IssueGraphFilter = "i-efgh:blocked-on:**".parse().unwrap();
        let query = SearchIssuesQuery {
            issue_type: None,
            status: None,
            assignee: None,
            q: None,
            graph_filters: vec![filter1, filter2],
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("graph").map(String::as_str),
            Some("*:child-of:i-abcd,i-efgh:blocked-on:**")
        );
    }

    #[test]
    fn search_issues_query_serializes_empty_query() {
        let query = SearchIssuesQuery::default();

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("graph").map(String::as_str), Some(""));
        assert_eq!(
            params.len(),
            1,
            "only the graph filter key should exist when no filters are provided"
        );
    }

    #[test]
    fn issue_todo_list_defaults_when_missing() {
        let raw = r#"{"type":"task","description":"write docs"}"#;

        let issue: Issue = serde_json::from_str(raw).expect("issue should deserialize");

        assert!(issue.todo_list.is_empty());
        assert_eq!(issue.status, IssueStatus::Open);
    }

    #[test]
    fn issue_todo_list_round_trips_in_order() {
        let todos = vec![
            TodoItem {
                description: "first".to_string(),
                is_done: false,
            },
            TodoItem {
                description: "second".to_string(),
                is_done: true,
            },
        ];
        let issue = Issue {
            issue_type: IssueType::Task,
            description: "with todos".to_string(),
            creator: String::new(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            todo_list: todos.clone(),
            dependencies: Vec::new(),
            patches: Vec::new(),
        };

        let value = serde_json::to_value(&issue).expect("issue should serialize");
        assert_eq!(value["todo_list"], json!(todos));

        let round_trip: Issue = serde_json::from_value(value).expect("issue should deserialize");
        assert_eq!(round_trip.todo_list, todos);
    }
}
