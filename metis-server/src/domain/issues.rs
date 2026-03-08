use super::users::Username;
use metis_common::api::v1 as api;
use metis_common::{IssueId, PatchId, RepoName};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueStatus {
    #[default]
    Open,
    InProgress,
    Closed,
    Dropped,
    Rejected,
    Failed,
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
        }
    }

    /// Returns true if this status represents a terminal state
    /// (Closed, Dropped, Rejected, or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IssueStatus::Closed
                | IssueStatus::Dropped
                | IssueStatus::Rejected
                | IssueStatus::Failed
        )
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
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Bug,
    Feature,
    Task,
    Chore,
    #[serde(rename = "merge-request")]
    MergeRequest,
    #[serde(rename = "review-request")]
    ReviewRequest,
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

impl IssueDependency {
    pub fn new(dependency_type: IssueDependencyType, issue_id: IssueId) -> Self {
        Self {
            dependency_type,
            issue_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "JobSettings::is_default")]
    pub job_settings: JobSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo_list: Vec<TodoItem>,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
    #[serde(default)]
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
        job_settings: Option<JobSettings>,
        todo_list: Vec<TodoItem>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
    ) -> Self {
        Self {
            issue_type,
            title,
            description,
            creator,
            progress,
            status,
            assignee,
            job_settings: job_settings.unwrap_or_default(),
            todo_list,
            dependencies,
            patches,
            deleted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct JobSettings {
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
}

impl JobSettings {
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IssueConversionError {
    #[error("invalid graph filter '{filter}': {reason}")]
    InvalidGraphFilter {
        filter: IssueGraphFilter,
        reason: String,
    },
}

impl From<api::issues::IssueStatus> for IssueStatus {
    fn from(value: api::issues::IssueStatus) -> Self {
        match value {
            api::issues::IssueStatus::Open => IssueStatus::Open,
            api::issues::IssueStatus::InProgress => IssueStatus::InProgress,
            api::issues::IssueStatus::Closed => IssueStatus::Closed,
            api::issues::IssueStatus::Dropped => IssueStatus::Dropped,
            api::issues::IssueStatus::Rejected => IssueStatus::Rejected,
            api::issues::IssueStatus::Failed => IssueStatus::Failed,
            _ => unreachable!("unsupported IssueStatus variant"),
        }
    }
}

impl From<IssueStatus> for api::issues::IssueStatus {
    fn from(value: IssueStatus) -> Self {
        match value {
            IssueStatus::Open => api::issues::IssueStatus::Open,
            IssueStatus::InProgress => api::issues::IssueStatus::InProgress,
            IssueStatus::Closed => api::issues::IssueStatus::Closed,
            IssueStatus::Dropped => api::issues::IssueStatus::Dropped,
            IssueStatus::Rejected => api::issues::IssueStatus::Rejected,
            IssueStatus::Failed => api::issues::IssueStatus::Failed,
        }
    }
}

impl From<api::issues::IssueType> for IssueType {
    fn from(value: api::issues::IssueType) -> Self {
        match value {
            api::issues::IssueType::Bug => IssueType::Bug,
            api::issues::IssueType::Feature => IssueType::Feature,
            api::issues::IssueType::Task => IssueType::Task,
            api::issues::IssueType::Chore => IssueType::Chore,
            api::issues::IssueType::MergeRequest => IssueType::MergeRequest,
            api::issues::IssueType::ReviewRequest => IssueType::ReviewRequest,
            _ => unreachable!("unsupported IssueType variant"),
        }
    }
}

impl From<IssueType> for api::issues::IssueType {
    fn from(value: IssueType) -> Self {
        match value {
            IssueType::Bug => api::issues::IssueType::Bug,
            IssueType::Feature => api::issues::IssueType::Feature,
            IssueType::Task => api::issues::IssueType::Task,
            IssueType::Chore => api::issues::IssueType::Chore,
            IssueType::MergeRequest => api::issues::IssueType::MergeRequest,
            IssueType::ReviewRequest => api::issues::IssueType::ReviewRequest,
        }
    }
}

impl From<api::issues::IssueDependencyType> for IssueDependencyType {
    fn from(value: api::issues::IssueDependencyType) -> Self {
        match value {
            api::issues::IssueDependencyType::ChildOf => IssueDependencyType::ChildOf,
            api::issues::IssueDependencyType::BlockedOn => IssueDependencyType::BlockedOn,
            _ => unreachable!("unsupported IssueDependencyType variant"),
        }
    }
}

impl From<IssueDependencyType> for api::issues::IssueDependencyType {
    fn from(value: IssueDependencyType) -> Self {
        match value {
            IssueDependencyType::ChildOf => api::issues::IssueDependencyType::ChildOf,
            IssueDependencyType::BlockedOn => api::issues::IssueDependencyType::BlockedOn,
        }
    }
}

impl From<api::issues::IssueDependency> for IssueDependency {
    fn from(value: api::issues::IssueDependency) -> Self {
        Self {
            dependency_type: value.dependency_type.into(),
            issue_id: value.issue_id,
        }
    }
}

impl From<IssueDependency> for api::issues::IssueDependency {
    fn from(value: IssueDependency) -> Self {
        api::issues::IssueDependency::new(value.dependency_type.into(), value.issue_id)
    }
}

impl From<api::issues::TodoItem> for TodoItem {
    fn from(value: api::issues::TodoItem) -> Self {
        Self {
            description: value.description,
            is_done: value.is_done,
        }
    }
}

impl From<TodoItem> for api::issues::TodoItem {
    fn from(value: TodoItem) -> Self {
        api::issues::TodoItem::new(value.description, value.is_done)
    }
}

impl From<api::issues::IssueGraphFilterSide> for IssueGraphFilterSide {
    fn from(value: api::issues::IssueGraphFilterSide) -> Self {
        match value {
            api::issues::IssueGraphFilterSide::Left => IssueGraphFilterSide::Left,
            api::issues::IssueGraphFilterSide::Right => IssueGraphFilterSide::Right,
            _ => unreachable!("unsupported IssueGraphFilterSide variant"),
        }
    }
}

impl From<IssueGraphFilterSide> for api::issues::IssueGraphFilterSide {
    fn from(value: IssueGraphFilterSide) -> Self {
        match value {
            IssueGraphFilterSide::Left => api::issues::IssueGraphFilterSide::Left,
            IssueGraphFilterSide::Right => api::issues::IssueGraphFilterSide::Right,
        }
    }
}

impl From<api::issues::IssueGraphWildcard> for IssueGraphWildcard {
    fn from(value: api::issues::IssueGraphWildcard) -> Self {
        match value {
            api::issues::IssueGraphWildcard::Immediate => IssueGraphWildcard::Immediate,
            api::issues::IssueGraphWildcard::Transitive => IssueGraphWildcard::Transitive,
            _ => unreachable!("unsupported IssueGraphWildcard variant"),
        }
    }
}

impl From<IssueGraphWildcard> for api::issues::IssueGraphWildcard {
    fn from(value: IssueGraphWildcard) -> Self {
        match value {
            IssueGraphWildcard::Immediate => api::issues::IssueGraphWildcard::Immediate,
            IssueGraphWildcard::Transitive => api::issues::IssueGraphWildcard::Transitive,
        }
    }
}

impl From<api::issues::IssueGraphSelector> for IssueGraphSelector {
    fn from(value: api::issues::IssueGraphSelector) -> Self {
        match value {
            api::issues::IssueGraphSelector::Issue(id) => IssueGraphSelector::Issue(id),
            api::issues::IssueGraphSelector::Wildcard(kind) => {
                IssueGraphSelector::Wildcard(kind.into())
            }
            _ => unreachable!("unsupported IssueGraphSelector variant"),
        }
    }
}

impl From<IssueGraphSelector> for api::issues::IssueGraphSelector {
    fn from(value: IssueGraphSelector) -> Self {
        match value {
            IssueGraphSelector::Issue(id) => api::issues::IssueGraphSelector::Issue(id),
            IssueGraphSelector::Wildcard(kind) => {
                api::issues::IssueGraphSelector::Wildcard(kind.into())
            }
        }
    }
}

impl From<api::issues::IssueGraphFilter> for IssueGraphFilter {
    fn from(value: api::issues::IssueGraphFilter) -> Self {
        IssueGraphFilter {
            lhs: value.lhs.into(),
            dependency_type: value.dependency_type.into(),
            rhs: value.rhs.into(),
        }
    }
}

impl TryFrom<IssueGraphFilter> for api::issues::IssueGraphFilter {
    type Error = IssueConversionError;

    fn try_from(value: IssueGraphFilter) -> Result<Self, Self::Error> {
        api::issues::IssueGraphFilter::new(
            value.lhs.clone().into(),
            value.dependency_type.into(),
            value.rhs.clone().into(),
        )
        .map_err(|reason| IssueConversionError::InvalidGraphFilter {
            filter: value,
            reason,
        })
    }
}

impl From<api::issues::JobSettings> for JobSettings {
    fn from(value: api::issues::JobSettings) -> Self {
        Self {
            repo_name: value.repo_name,
            remote_url: value.remote_url,
            image: value.image,
            model: value.model,
            branch: value.branch,
            max_retries: value.max_retries,
            cpu_limit: value.cpu_limit,
            memory_limit: value.memory_limit,
        }
    }
}

impl From<JobSettings> for api::issues::JobSettings {
    fn from(value: JobSettings) -> Self {
        let mut job_settings = api::issues::JobSettings::default();
        job_settings.repo_name = value.repo_name;
        job_settings.remote_url = value.remote_url;
        job_settings.image = value.image;
        job_settings.model = value.model;
        job_settings.branch = value.branch;
        job_settings.max_retries = value.max_retries;
        job_settings.cpu_limit = value.cpu_limit;
        job_settings.memory_limit = value.memory_limit;
        job_settings
    }
}

impl From<api::issues::Issue> for Issue {
    fn from(value: api::issues::Issue) -> Self {
        Self {
            issue_type: value.issue_type.into(),
            title: value.title,
            description: value.description,
            creator: value.creator.into(),
            progress: value.progress,
            status: value.status.into(),
            assignee: value.assignee,
            job_settings: value.job_settings.into(),
            todo_list: value.todo_list.into_iter().map(Into::into).collect(),
            dependencies: value.dependencies.into_iter().map(Into::into).collect(),
            patches: value.patches,
            deleted: value.deleted,
        }
    }
}

impl From<Issue> for api::issues::Issue {
    fn from(value: Issue) -> Self {
        api::issues::Issue::new(
            value.issue_type.into(),
            value.title,
            value.description,
            value.creator.into(),
            value.progress,
            value.status.into(),
            value.assignee,
            Some(value.job_settings.into()),
            value.todo_list.into_iter().map(Into::into).collect(),
            value.dependencies.into_iter().map(Into::into).collect(),
            value.patches,
            value.deleted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::users::Username;
    use metis_common::api::v1 as api;
    use metis_common::{IssueId, PatchId, RepoName};
    use std::str::FromStr;

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
    fn issue_roundtrip_json() {
        let dependency_id = IssueId::new();
        let patch_id = PatchId::new();
        let job_settings = JobSettings {
            repo_name: Some(RepoName::from_str("dourolabs/metis").unwrap()),
            remote_url: Some("https://github.com/dourolabs/metis".to_string()),
            image: Some("worker:latest".to_string()),
            model: Some("gpt-4o".to_string()),
            branch: Some("main".to_string()),
            max_retries: Some(2),
            cpu_limit: Some("400m".to_string()),
            memory_limit: Some("768Mi".to_string()),
        };
        let issue = Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: "cool feature".to_string(),
            creator: Username::from("alice"),
            progress: "in-progress".to_string(),
            status: IssueStatus::Open,
            assignee: Some("bob".to_string()),
            job_settings: job_settings.clone(),
            todo_list: vec![TodoItem {
                description: "todo".to_string(),
                is_done: false,
            }],
            dependencies: vec![IssueDependency {
                dependency_type: IssueDependencyType::ChildOf,
                issue_id: dependency_id,
            }],
            patches: vec![patch_id.clone()],
            deleted: false,
        };

        let issue_json = serde_json::to_string(&issue).expect("should serialize to JSON");
        let decoded: Issue = serde_json::from_str(&issue_json).expect("should parse issue");

        assert_eq!(
            decoded.dependencies[0].dependency_type,
            IssueDependencyType::ChildOf
        );
        assert_eq!(decoded.patches[0], patch_id);
        assert_eq!(decoded.creator, issue.creator);
        assert_eq!(decoded.assignee, Some("bob".to_string()));
        assert_eq!(decoded.status, issue.status);
        assert_eq!(decoded.progress, issue.progress);
        assert_eq!(decoded.issue_type, issue.issue_type);
        assert_eq!(decoded.description, issue.description);
        assert_eq!(decoded.todo_list.len(), 1);
        assert_eq!(decoded.job_settings, job_settings);
    }

    #[test]
    fn issue_graph_filter_conversion_rejects_missing_wildcard() {
        let left = IssueId::new();
        let right = IssueId::new();
        let filter = IssueGraphFilter::new(
            IssueGraphSelector::Issue(left),
            IssueDependencyType::ChildOf,
            IssueGraphSelector::Issue(right),
        );

        let result = api::issues::IssueGraphFilter::try_from(filter);
        assert!(matches!(
            result,
            Err(IssueConversionError::InvalidGraphFilter { .. })
        ));
    }

    #[test]
    fn issue_graph_filter_converts_with_single_wildcard() {
        let issue_id = IssueId::new();
        let filter = IssueGraphFilter::new(
            IssueGraphSelector::Wildcard(IssueGraphWildcard::Immediate),
            IssueDependencyType::BlockedOn,
            IssueGraphSelector::Issue(issue_id),
        );

        let api_filter: api::issues::IssueGraphFilter =
            api::issues::IssueGraphFilter::try_from(filter.clone())
                .expect("conversion should work");

        assert_eq!(api_filter.to_string(), filter.to_string());
    }
}
