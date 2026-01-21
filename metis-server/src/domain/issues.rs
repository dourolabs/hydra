use metis_common::{IssueId, PatchId, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
pub struct IssueDependency {
    #[serde(rename = "type")]
    pub dependency_type: IssueDependencyType,
    pub issue_id: IssueId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub description: String,
    #[serde(default)]
    pub is_done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoListResponse {
    pub issue_id: IssueId,
    #[serde(default)]
    pub todo_list: Vec<TodoItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddTodoItemRequest {
    pub description: String,
    #[serde(default)]
    pub is_done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceTodoListRequest {
    #[serde(default)]
    pub todo_list: Vec<TodoItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTodoItemStatusRequest {
    pub is_done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueGraphFilterSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

        if trimmed == IssueGraphWildcard::Immediate.as_str() {
            return Ok(IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate));
        }

        if trimmed == IssueGraphWildcard::Transitive.as_str() {
            return Ok(IssueGraphSelector::Wildcard(IssueGraphWildcard::Transitive));
        }

        let issue_id = IssueId::from_str(trimmed)
            .map_err(|err| format!("invalid issue id in graph filter: {err}"))?;
        Ok(IssueGraphSelector::Issue(issue_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    ) -> Self {
        Self {
            lhs,
            dependency_type,
            rhs,
        }
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
        write!(
            f,
            "{}:{}:{}",
            self.lhs.as_str(),
            self.dependency_type.as_str(),
            self.rhs.as_str()
        )
    }
}

impl IssueGraphSelector {
    pub fn as_str(&self) -> String {
        match self {
            IssueGraphSelector::Issue(issue_id) => issue_id.as_ref().to_string(),
            IssueGraphSelector::Wildcard(wildcard) => wildcard.as_str().to_string(),
        }
    }
}

impl FromStr for IssueGraphFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return Err("invalid graph filter format, expected 'lhs:dependency:rhs'".to_string());
        }

        let lhs = parts[0];
        let dependency_type = parts[1];
        let rhs = parts[2];

        let lhs = IssueGraphSelector::parse(lhs)?;
        let rhs = IssueGraphSelector::parse(rhs)?;
        let dependency_type = IssueDependencyType::from_str(dependency_type)
            .map_err(|err| format!("invalid dependency type in graph filter: {err}"))?;
        Ok(IssueGraphFilter::new(lhs, dependency_type, rhs))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRecord {
    pub id: IssueId,
    pub issue: Issue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertIssueRequest {
    pub issue: Issue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertIssueResponse {
    pub issue_id: IssueId,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListIssuesResponse {
    pub issues: Vec<IssueRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::{IssueId, PatchId, TaskId};
    use serde::Serialize;
    use serde_json::json;
    use std::collections::HashMap;

    fn serialize_query_params<T: Serialize>(value: &T) -> Vec<(String, String)> {
        let encoded = serde_urlencoded::to_string(value).unwrap();
        serde_urlencoded::from_str(&encoded).unwrap()
    }

    #[test]
    fn issue_graph_filters_roundtrip() {
        let left = IssueId::new();
        let right = IssueId::new();
        let raw = format!("{}:blocked-on:{}", left.as_ref(), right.as_ref());
        let filter = IssueGraphFilter::from_str(&raw).expect("should parse filter");
        assert_eq!(filter.to_string(), raw);
    }

    #[test]
    fn issue_graph_filters_roundtrip_wildcards() {
        let raw = "**:blocked-on:*";
        let filter = IssueGraphFilter::from_str(raw).expect("should parse filter");
        assert_eq!(filter.to_string(), raw);
    }

    #[test]
    fn issue_graph_filters_fail_on_empty_selector() {
        let filter = IssueGraphFilter::from_str("  :blocked-on:*");
        assert!(filter.is_err());
    }

    #[test]
    fn search_query_deserialization() {
        let issue_id = IssueId::new();
        let filters = [format!(
            "{}:{}:**",
            issue_id.as_ref(),
            IssueDependencyType::ChildOf.as_str()
        )];

        let json_value = json!({
            "issue_type": "feature",
            "status": "open",
            "assignee": "alice",
            "q": "some query",
            "graph": filters.join(","),
        });

        let query: SearchIssuesQuery =
            serde_json::from_value(json_value).expect("JSON should be valid");

        assert_eq!(query.issue_type, Some(IssueType::Feature));
        assert_eq!(query.status, Some(IssueStatus::Open));
        assert_eq!(query.assignee, Some("alice".to_string()));
        assert_eq!(query.q, Some("some query".to_string()));
        assert_eq!(query.graph_filters.len(), 1);
        assert_eq!(
            query.graph_filters[0].to_string(),
            format!(
                "{}:{}:**",
                issue_id.as_ref(),
                IssueDependencyType::ChildOf.as_str()
            )
        );
    }

    #[test]
    fn search_issues_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let filter = format!(
            "{}:{}:**",
            issue_id.as_ref(),
            IssueDependencyType::ChildOf.as_str()
        );
        let query = SearchIssuesQuery {
            issue_type: Some(IssueType::Feature),
            status: Some(IssueStatus::Open),
            assignee: Some("alice".to_string()),
            q: Some("test query".to_string()),
            graph_filters: vec![IssueGraphFilter::from_str(&filter).unwrap()],
        };

        let params = serialize_query_params(&query);
        let params: HashMap<_, _> = params.into_iter().collect();

        assert_eq!(
            params.get("issue_type").map(String::as_str),
            Some("feature")
        );
        assert_eq!(params.get("status").map(String::as_str), Some("open"));
        assert_eq!(params.get("assignee").map(String::as_str), Some("alice"));
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
        assert_eq!(
            params.get("graph").map(String::as_str),
            Some(filter.as_str())
        );
    }

    #[test]
    fn upsert_issue_request_roundtrip_json() {
        let dependency_id = IssueId::new();
        let patch_id = PatchId::new();
        let payload = UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "cool feature".to_string(),
                creator: "alice".to_string(),
                progress: "in-progress".to_string(),
                status: IssueStatus::Open,
                assignee: Some("bob".to_string()),
                todo_list: vec![TodoItem {
                    description: "todo".to_string(),
                    is_done: false,
                }],
                dependencies: vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: dependency_id,
                }],
                patches: vec![patch_id.clone()],
            },
            job_id: Some(TaskId::new()),
        };

        let payload_json = serde_json::to_string(&payload).expect("should serialize to JSON");
        let decoded: UpsertIssueRequest =
            serde_json::from_str(&payload_json).expect("should parse payload");

        assert_eq!(
            decoded.issue.dependencies[0].dependency_type,
            IssueDependencyType::ChildOf
        );
        assert_eq!(decoded.issue.patches[0], patch_id);
        assert_eq!(decoded.job_id, payload.job_id);
        assert_eq!(decoded.issue.creator, payload.issue.creator);
        assert_eq!(decoded.issue.assignee, Some("bob".to_string()));
        assert_eq!(decoded.issue.status, payload.issue.status);
        assert_eq!(decoded.issue.progress, payload.issue.progress);
        assert_eq!(decoded.issue.issue_type, payload.issue.issue_type);
        assert_eq!(decoded.issue.description, payload.issue.description);
        assert_eq!(decoded.issue.todo_list.len(), 1);
    }
}
