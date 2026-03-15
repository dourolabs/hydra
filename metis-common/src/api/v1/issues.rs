use super::labels::LabelSummary;
use super::users::Username;
pub use crate::IssueId;
use crate::{LabelId, PatchId, RepoName, SessionId, VersionNumber, actor_ref::ActorRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IssueStatus {
    #[default]
    Open,
    InProgress,
    Closed,
    Dropped,
    Rejected,
    Failed,
    #[serde(other)]
    Unknown,
}

impl IssueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueStatus::Open => "open",
            IssueStatus::InProgress => "in-progress",
            IssueStatus::Closed => "closed",
            IssueStatus::Dropped => "dropped",
            IssueStatus::Rejected => "rejected",
            IssueStatus::Failed => "failed",
            IssueStatus::Unknown => "unknown",
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
            "rejected" => Ok(IssueStatus::Rejected),
            "failed" => Ok(IssueStatus::Failed),
            other => Err(format!("unsupported issue status '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueType {
    Bug,
    Feature,
    Task,
    Chore,
    #[serde(rename = "merge-request")]
    MergeRequest,
    #[serde(rename = "review-request")]
    ReviewRequest,
    #[serde(other)]
    Unknown,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::Bug => "bug",
            IssueType::Feature => "feature",
            IssueType::Task => "task",
            IssueType::Chore => "chore",
            IssueType::MergeRequest => "merge-request",
            IssueType::ReviewRequest => "review-request",
            IssueType::Unknown => "unknown",
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
            "review-request" | "reviewrequest" | "review_request" => Ok(IssueType::ReviewRequest),
            other => Err(format!("unsupported issue type '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IssueDependencyType {
    ChildOf,
    BlockedOn,
    #[serde(other)]
    Unknown,
}

impl IssueDependencyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueDependencyType::ChildOf => "child-of",
            IssueDependencyType::BlockedOn => "blocked-on",
            IssueDependencyType::Unknown => "unknown",
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Issue {
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub creator: Username,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub status: IssueStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<String>,
    #[serde(
        default,
        alias = "job_settings",
        skip_serializing_if = "SessionSettings::is_default"
    )]
    pub session_settings: SessionSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo_list: Vec<TodoItem>,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Issue {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issue_type: IssueType,
        title: String,
        description: String,
        creator: Username,
        progress: String,
        status: IssueStatus,
        assignee: Option<String>,
        session_settings: Option<SessionSettings>,
        todo_list: Vec<TodoItem>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
        deleted: bool,
    ) -> Self {
        Self {
            issue_type,
            title,
            description,
            creator,
            progress,
            status,
            assignee,
            session_settings: session_settings.unwrap_or_default(),
            todo_list,
            dependencies,
            patches,
            deleted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSettings {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repo_name: Option<RepoName>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub remote_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cpu_limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub memory_limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub secrets: Option<Vec<String>>,
}

impl SessionSettings {
    pub fn is_default(value: &Self) -> bool {
        value == &Self::default()
    }

    pub fn merge(mut primary: Self, mut secondary: Self) -> Self {
        primary.apply_owned(&mut secondary);
        primary
    }

    fn apply_owned(&mut self, other: &mut Self) {
        if self.repo_name.is_none() {
            self.repo_name = other.repo_name.take();
        }
        if self.remote_url.is_none() {
            self.remote_url = other.remote_url.take();
        }
        if self.image.is_none() {
            self.image = other.image.take();
        }
        if self.model.is_none() {
            self.model = other.model.take();
        }
        if self.branch.is_none() {
            self.branch = other.branch.take();
        }
        if self.max_retries.is_none() {
            self.max_retries = other.max_retries.take();
        }
        if self.cpu_limit.is_none() {
            self.cpu_limit = other.cpu_limit.take();
        }
        if self.memory_limit.is_none() {
            self.memory_limit = other.memory_limit.take();
        }
        if self.secrets.is_none() {
            self.secrets = other.secrets.take();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssueVersionRecord {
    pub issue_id: IssueId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub issue: Issue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelSummary>,
}

impl IssueVersionRecord {
    pub fn new(
        issue_id: IssueId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        issue: Issue,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
        labels: Vec<LabelSummary>,
    ) -> Self {
        Self {
            issue_id,
            version,
            timestamp,
            issue,
            actor,
            creation_time,
            labels,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertIssueRequest {
    pub issue: Issue,
    #[serde(skip_serializing_if = "Option::is_none", alias = "job_id")]
    pub session_id: Option<SessionId>,
    /// Label IDs to associate with this issue (replaces existing labels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<LabelId>>,
    /// Label names to associate with this issue (resolved to label IDs).
    /// Labels that do not exist are created automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_names: Option<Vec<String>>,
}

impl UpsertIssueRequest {
    pub fn new(issue: Issue, session_id: Option<SessionId>) -> Self {
        Self {
            issue,
            session_id,
            label_ids: None,
            label_names: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertIssueResponse {
    pub issue_id: IssueId,
    pub version: VersionNumber,
}

impl UpsertIssueResponse {
    pub fn new(issue_id: IssueId, version: VersionNumber) -> Self {
        Self { issue_id, version }
    }
}

use super::serde_helpers::{deserialize_comma_separated, serialize_comma_separated};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchIssuesQuery {
    /// Batch-fetch specific issues by ID (comma-separated, max 100).
    /// Intersected with other filters when provided.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub ids: Vec<IssueId>,
    #[serde(default)]
    pub issue_type: Option<IssueType>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub status: Vec<IssueStatus>,
    #[serde(default)]
    pub assignee: Option<String>,
    /// Filter issues by creator username.
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(
        default,
        rename = "graph",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub graph_filters: Vec<IssueGraphFilter>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
    /// Filter issues by label IDs (comma-separated in query string).
    #[serde(
        default,
        rename = "labels",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub label_ids: Vec<LabelId>,
    /// Maximum number of results to return. When omitted, all results are returned.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
    /// When true, include `total_count` in the response.
    #[serde(default)]
    pub count: Option<bool>,
}

impl SearchIssuesQuery {
    pub fn new(
        issue_type: Option<IssueType>,
        status: Vec<IssueStatus>,
        assignee: Option<String>,
        q: Option<String>,
        graph_filters: Vec<IssueGraphFilter>,
        include_deleted: Option<bool>,
    ) -> Self {
        Self {
            ids: Vec::new(),
            issue_type,
            status,
            assignee,
            creator: None,
            q,
            graph_filters,
            include_deleted,
            label_ids: Vec::new(),
            limit: None,
            cursor: None,
            count: None,
        }
    }
}

/// Lightweight summary of an issue for list views.
///
/// Excludes `session_settings` and the full `description` body.
/// The `description` field is truncated to the first line (max 200 chars).
/// The `progress` field is truncated to the first 200 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssueSummary {
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub creator: Username,
    #[serde(default)]
    pub status: IssueStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo_list: Vec<TodoItem>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelSummary>,
}

impl From<&Issue> for IssueSummary {
    fn from(issue: &Issue) -> Self {
        let first_line = issue.description.lines().next().unwrap_or("");
        let truncated = if first_line.len() > 200 {
            first_line.chars().take(200).collect()
        } else {
            first_line.to_string()
        };
        let progress = if issue.progress.len() > 200 {
            issue.progress.chars().take(200).collect()
        } else {
            issue.progress.clone()
        };
        IssueSummary {
            issue_type: issue.issue_type,
            title: issue.title.clone(),
            description: truncated,
            creator: issue.creator.clone(),
            status: issue.status,
            assignee: issue.assignee.clone(),
            progress,
            dependencies: issue.dependencies.clone(),
            patches: issue.patches.clone(),
            todo_list: issue.todo_list.clone(),
            deleted: issue.deleted,
            labels: Vec::new(),
        }
    }
}

/// Summary-level version record for issue list responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssueSummaryRecord {
    pub issue_id: IssueId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub issue: IssueSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
}

impl IssueSummaryRecord {
    pub fn new(
        issue_id: IssueId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        issue: IssueSummary,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
        labels: Vec<LabelSummary>,
    ) -> Self {
        let mut issue = issue;
        issue.labels = labels;
        Self {
            issue_id,
            version,
            timestamp,
            issue,
            actor,
            creation_time,
        }
    }
}

impl From<&IssueVersionRecord> for IssueSummaryRecord {
    fn from(record: &IssueVersionRecord) -> Self {
        let mut summary = IssueSummary::from(&record.issue);
        summary.labels = record.labels.clone();
        IssueSummaryRecord {
            issue_id: record.issue_id.clone(),
            version: record.version,
            timestamp: record.timestamp,
            issue: summary,
            actor: record.actor.clone(),
            creation_time: record.creation_time,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListIssuesResponse {
    pub issues: Vec<IssueSummaryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

impl ListIssuesResponse {
    pub fn new(issues: Vec<IssueSummaryRecord>) -> Self {
        Self {
            issues,
            next_cursor: None,
            total_count: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListIssueVersionsResponse {
    pub versions: Vec<IssueVersionRecord>,
}

impl ListIssueVersionsResponse {
    pub fn new(versions: Vec<IssueVersionRecord>) -> Self {
        Self { versions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use crate::users::Username;
    use serde_json::json;
    use std::{collections::HashMap, str::FromStr};

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
            ids: vec![],
            issue_type: Some(IssueType::Bug),
            status: vec![IssueStatus::Open],
            assignee: Some("alice".to_string()),
            creator: None,
            q: Some("test query".to_string()),
            graph_filters: vec![],
            include_deleted: None,
            label_ids: vec![],
            limit: None,
            cursor: None,
            count: None,
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
            ids: vec![],
            issue_type: None,
            status: vec![],
            assignee: None,
            creator: None,
            q: None,
            graph_filters: vec![filter1, filter2],
            include_deleted: None,
            label_ids: vec![],
            limit: None,
            cursor: None,
            count: None,
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
        assert_eq!(params.get("labels").map(String::as_str), Some(""));
        assert_eq!(
            params.len(),
            2,
            "only the graph and labels keys should exist when no filters are provided"
        );
    }

    #[test]
    fn search_issues_query_serializes_ids() {
        let query = SearchIssuesQuery {
            ids: vec![issue_id("i-abcd"), issue_id("i-efgh")],
            ..SearchIssuesQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("ids").map(String::as_str), Some("i-abcd,i-efgh"));
    }

    #[test]
    fn search_issues_query_serializes_multi_status() {
        let query = SearchIssuesQuery {
            status: vec![IssueStatus::Open, IssueStatus::InProgress],
            ..SearchIssuesQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("status").map(String::as_str),
            Some("open,in-progress")
        );
    }

    #[test]
    fn search_issues_query_omits_empty_status() {
        let query = SearchIssuesQuery::default();
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!(
            params.get("status").is_none(),
            "empty status vec should be omitted from serialization"
        );
    }

    #[test]
    fn search_issues_query_deserializes_ids() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("ids=i-abcd%2Ci-efgh").unwrap();
        assert_eq!(query.ids.len(), 2);
        assert_eq!(query.ids[0].as_ref(), "i-abcd");
        assert_eq!(query.ids[1].as_ref(), "i-efgh");
    }

    #[test]
    fn search_issues_query_serializes_creator() {
        let query = SearchIssuesQuery {
            creator: Some("alice".to_string()),
            ..SearchIssuesQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("creator").map(String::as_str), Some("alice"));
    }

    #[test]
    fn issue_todo_list_defaults_when_missing() {
        let raw = r#"{"type":"task","description":"write docs","creator":"alice"}"#;

        let issue: Issue = serde_json::from_str(raw).expect("issue should deserialize");

        assert!(issue.todo_list.is_empty());
        assert_eq!(issue.status, IssueStatus::Open);
        assert!(SessionSettings::is_default(&issue.session_settings));
    }

    #[test]
    fn issue_todo_list_round_trips_in_order() {
        let session_settings = SessionSettings {
            repo_name: Some(RepoName::from_str("dourolabs/metis").unwrap()),
            remote_url: Some("https://github.com/dourolabs/metis".to_string()),
            image: Some("worker:latest".to_string()),
            model: Some("gpt-4o".to_string()),
            branch: Some("main".to_string()),
            max_retries: Some(3),
            cpu_limit: Some("500m".to_string()),
            memory_limit: Some("1Gi".to_string()),
            secrets: Some(vec!["my-secret".to_string()]),
        };
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
            title: String::new(),
            description: "with todos".to_string(),
            creator: Username::from("author"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            session_settings: session_settings.clone(),
            todo_list: todos.clone(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
        };

        let value = serde_json::to_value(&issue).expect("issue should serialize");
        assert_eq!(value["todo_list"], json!(todos));

        let round_trip: Issue = serde_json::from_value(value).expect("issue should deserialize");
        assert_eq!(round_trip.todo_list, todos);
        assert_eq!(round_trip.session_settings, session_settings);
    }

    #[test]
    fn issue_version_record_serializes_actor_when_present() {
        use crate::actor_ref::{ActorId, ActorRef};

        let issue_id: IssueId = "i-test".parse().unwrap();
        let issue = Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: "test".to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            session_settings: Default::default(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
        };

        let actor = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
        };
        let ts = chrono::Utc::now();
        let record =
            IssueVersionRecord::new(issue_id, 1, ts, issue, Some(actor.clone()), ts, Vec::new());

        let value = serde_json::to_value(&record).expect("should serialize");
        let expected_actor = json!({"Authenticated": {"actor_id": {"Username": "alice"}}});
        assert_eq!(value["actor"], expected_actor);
    }

    #[test]
    fn issue_version_record_omits_actor_when_none() {
        let issue_id: IssueId = "i-test".parse().unwrap();
        let issue = Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: "test".to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            session_settings: Default::default(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
        };

        let ts = chrono::Utc::now();
        let record = IssueVersionRecord::new(issue_id, 1, ts, issue, None, ts, Vec::new());

        let value = serde_json::to_value(&record).expect("should serialize");
        assert!(
            value.get("actor").is_none(),
            "actor should be omitted when None"
        );
    }

    #[test]
    fn issue_version_record_deserializes_without_actor() {
        let json = r#"{
            "issue_id": "i-test",
            "version": 1,
            "timestamp": "2024-01-01T00:00:00Z",
            "issue": {"type": "task", "description": "test", "creator": "alice"},
            "creation_time": "2024-01-01T00:00:00Z"
        }"#;

        let record: IssueVersionRecord =
            serde_json::from_str(json).expect("should deserialize without actor");
        assert_eq!(record.actor, None);
    }

    fn make_test_issue(description: &str) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: description.to_string(),
            creator: Username::from("alice"),
            progress: "some progress text".to_string(),
            status: IssueStatus::InProgress,
            assignee: Some("bob".to_string()),
            session_settings: SessionSettings {
                repo_name: Some(RepoName::from_str("org/repo").unwrap()),
                ..Default::default()
            },
            todo_list: vec![TodoItem {
                description: "do something".to_string(),
                is_done: false,
            }],
            dependencies: vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                issue_id("i-parent"),
            )],
            patches: vec!["p-abcd".parse().unwrap()],
            deleted: false,
        }
    }

    #[test]
    fn issue_summary_truncates_description_to_first_line() {
        let issue = make_test_issue("First line\nSecond line\nThird line");
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.description, "First line");
    }

    #[test]
    fn issue_summary_truncates_long_first_line() {
        let long_line = "a".repeat(300);
        let issue = make_test_issue(&long_line);
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.description.len(), 200);
        assert!(summary.description.chars().all(|c| c == 'a'));
    }

    #[test]
    fn issue_summary_preserves_short_description() {
        let issue = make_test_issue("short desc");
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.description, "short desc");
    }

    #[test]
    fn issue_summary_truncates_long_progress() {
        let mut issue = make_test_issue("desc");
        issue.progress = "p".repeat(300);
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.progress.len(), 200);
        assert!(summary.progress.chars().all(|c| c == 'p'));
    }

    #[test]
    fn issue_summary_excludes_session_settings() {
        let issue = make_test_issue("test issue");
        let summary = IssueSummary::from(&issue);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("session_settings").is_none());
    }

    #[test]
    fn issue_summary_maps_all_fields() {
        let issue = make_test_issue("desc");
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.issue_type, IssueType::Task);
        assert_eq!(summary.creator, Username::from("alice"));
        assert_eq!(summary.progress, "some progress text");
        assert_eq!(summary.status, IssueStatus::InProgress);
        assert_eq!(summary.assignee.as_deref(), Some("bob"));
        assert_eq!(summary.dependencies.len(), 1);
        assert_eq!(summary.patches.len(), 1);
        assert_eq!(summary.todo_list.len(), 1);
        assert!(!summary.deleted);
    }

    #[test]
    fn issue_summary_record_from_version_record() {
        let issue = make_test_issue("multi\nline\ndesc");
        let ts = chrono::Utc::now();
        let record =
            IssueVersionRecord::new(issue_id("i-test"), 3, ts, issue, None, ts, Vec::new());
        let summary_record = IssueSummaryRecord::from(&record);
        assert_eq!(summary_record.issue_id, issue_id("i-test"));
        assert_eq!(summary_record.version, 3);
        assert_eq!(summary_record.issue.description, "multi");
        assert_eq!(summary_record.actor, None);
    }

    #[test]
    fn issue_summary_handles_empty_description() {
        let issue = make_test_issue("");
        let summary = IssueSummary::from(&issue);
        assert_eq!(summary.description, "");
    }
}
