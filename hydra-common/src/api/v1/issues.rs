use super::form::{Form, FormResponse};
use super::labels::LabelSummary;
use super::projects::{StatusDefinition, StatusKey};
use super::users::Username;
pub use crate::IssueId;
use crate::principal::Principal;
use crate::{LabelId, PatchId, ProjectId, RepoName, SessionId, VersionNumber, actor_ref::ActorRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{collections::HashMap, fmt, str::FromStr};

/// Default value for `Issue.status` and `IssueSummary.status` when the field
/// is omitted from a request body. Matches the historical "open" landing
/// status of the synthesized `DefaultProject` so existing clients that don't
/// set a status still land in `open` exactly like they did before the PR 3
/// wire-shape change.
fn default_status_key() -> StatusKey {
    StatusKey::try_new("open").expect("\"open\" is a well-formed StatusKey")
}

/// Serialize an `Option<Principal>` as its canonical path form
/// (`users/<x>` / `agents/<x>` / `external/<sys>/<x>`) so it survives URL
/// query-string encoding. Used by [`SearchIssuesQuery::assignee`].
fn serialize_option_principal_path<S: Serializer>(
    value: &Option<Principal>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(p) => serializer.serialize_str(&p.to_path()),
        None => serializer.serialize_none(),
    }
}

/// Deserialize an `Option<Principal>` from its canonical path form. A missing
/// query param deserializes to `None`; an explicit empty string is rejected
/// (it would not parse as a Principal). This keeps "no filter" and "empty
/// string" distinguishable on the wire.
fn deserialize_option_principal_path<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Principal>, D::Error> {
    let raw: Option<String> = Option::deserialize(deserializer)?;
    match raw {
        Some(s) => Principal::from_str(&s)
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

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
            IssueStatus::Failed => "failed",
            IssueStatus::Unknown => "unknown",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IssueStatus::Closed | IssueStatus::Dropped | IssueStatus::Failed
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(self, IssueStatus::Open | IssueStatus::InProgress)
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
            // Backward-compat: old "rejected" wire values deserialize to Dropped; remove once the 2026-05-08 migration has soaked.
            "rejected" => Ok(IssueStatus::Dropped),
            "failed" => Ok(IssueStatus::Failed),
            other => Err(format!("unsupported issue status '{other}'")),
        }
    }
}

impl From<IssueStatus> for StatusKey {
    /// Legacy adapter: returns the wire string of the enum variant as a
    /// [`StatusKey`]. Always succeeds (the five legacy strings are
    /// well-formed keys by construction). `Unknown` falls back to
    /// `unknown` which is also a valid key.
    fn from(value: IssueStatus) -> Self {
        StatusKey::try_new(value.as_str()).expect("IssueStatus wire string is a valid StatusKey")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IssueType {
    Bug,
    Feature,
    Task,
    Chore,
    MergeRequest,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Status key for this issue; resolved against its project's status
    /// list (or [`super::projects::Project`]'s synthesized default project
    /// when [`Self::project_id`] is None). The wire string is unchanged
    /// for the five legacy statuses (`open`, `in-progress`, `closed`,
    /// `dropped`, `failed`) so older clients keep working.
    #[serde(default = "default_status_key")]
    pub status: StatusKey,
    /// Optional project this issue belongs to. When None, the issue
    /// resolves through the synthesized default project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    /// Server-computed status definition (display props + dependency
    /// flags) for [`Self::status`], resolved against the issue's project's
    /// status list. Never stored: always populated on responses so
    /// frontends don't need a second round trip to render the status
    /// chip, and omitted on create / update requests (the server
    /// re-resolves from [`Self::status`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_status: Option<StatusDefinition>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<Principal>,
    #[serde(
        default,
        alias = "job_settings",
        skip_serializing_if = "SessionSettings::is_default"
    )]
    pub session_settings: SessionSettings,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<Form>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_response: Option<FormResponse>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub feedback: Option<String>,
}

impl Issue {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issue_type: IssueType,
        title: String,
        description: String,
        creator: Username,
        progress: String,
        status: StatusKey,
        project_id: Option<ProjectId>,
        assignee: Option<Principal>,
        session_settings: Option<SessionSettings>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
        deleted: bool,
        form: Option<Form>,
        form_response: Option<FormResponse>,
        feedback: Option<String>,
    ) -> Self {
        Self {
            issue_type,
            title,
            description,
            creator,
            progress,
            status,
            project_id,
            resolved_status: None,
            assignee,
            session_settings: session_settings.unwrap_or_default(),
            dependencies,
            patches,
            deleted,
            form,
            form_response,
            feedback,
        }
    }
}

impl crate::graph::GraphView for Issue {
    const KIND: crate::graph::ObjectKind = crate::graph::ObjectKind::Issue;

    fn view_l1(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "status": self.status.as_str(),
        })
    }

    fn view_l2(&self) -> Value {
        let progress = if self.progress.chars().count() > 200 {
            let mut truncated: String = self.progress.chars().take(200).collect();
            truncated.push_str("...");
            truncated
        } else {
            self.progress.clone()
        };
        serde_json::json!({
            "title": self.title,
            "status": self.status.as_str(),
            "assignee": self.assignee,
            "progress": progress,
            "dependencies": self.dependencies,
        })
    }

    fn view_l3(&self) -> Value {
        serde_json::to_value(self).expect("Issue serialization is infallible")
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Comma-separated list of [`StatusKey`] strings to filter on.
    ///
    /// `StatusKey` is a transparent string newtype, so the five legacy
    /// `IssueStatus` strings (`open`, `in-progress`, `closed`, `dropped`,
    /// `failed`) and per-project keys (e.g. `inbox`, `triage`) share the
    /// same wire shape.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub status: Vec<StatusKey>,
    /// Scope results to a specific project. Omit to span all projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(
        default,
        serialize_with = "serialize_option_principal_path",
        deserialize_with = "deserialize_option_principal_path"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub assignee: Option<Principal>,
    /// Filter issues by creator username.
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
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
        status: Vec<StatusKey>,
        assignee: Option<Principal>,
        q: Option<String>,
        include_deleted: Option<bool>,
    ) -> Self {
        Self {
            ids: Vec::new(),
            issue_type,
            status,
            project_id: None,
            assignee,
            creator: None,
            q,
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
    #[serde(default = "default_status_key")]
    pub status: StatusKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    /// Server-computed status definition; populated by the route handler
    /// before serialization (omitted on requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_status: Option<StatusDefinition>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<Principal>,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
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
            status: issue.status.clone(),
            project_id: issue.project_id.clone(),
            resolved_status: issue.resolved_status.clone(),
            assignee: issue.assignee.clone(),
            progress,
            dependencies: issue.dependencies.clone(),
            patches: issue.patches.clone(),
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Request body for POST /v1/issues/{issue_id}/actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SubmitFormRequest {
    /// Which action button was clicked.
    pub action_id: String,

    /// Collected field values, keyed by field key.
    #[serde(default)]
    pub values: HashMap<String, Value>,
}

impl SubmitFormRequest {
    pub fn new(action_id: String, values: HashMap<String, Value>) -> Self {
        Self { action_id, values }
    }
}

/// Response body for a successful form submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SubmitFormResponse {
    pub issue_id: IssueId,
    pub version: VersionNumber,
    pub form_response: FormResponse,
}

impl SubmitFormResponse {
    pub fn new(issue_id: IssueId, version: VersionNumber, form_response: FormResponse) -> Self {
        Self {
            issue_id,
            version,
            form_response,
        }
    }
}

/// Request body for POST /v1/issues/{issue_id}/feedback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SubmitFeedbackRequest {
    pub feedback: String,
}

impl SubmitFeedbackRequest {
    pub fn new(feedback: String) -> Self {
        Self { feedback }
    }
}

/// Structured validation error response for form submissions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct FormValidationError {
    pub error: String,
    pub field_errors: HashMap<String, String>,
}

impl FormValidationError {
    pub fn new(field_errors: HashMap<String, String>) -> Self {
        Self {
            error: "validation_failed".to_string(),
            field_errors,
        }
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
    fn search_issues_query_serializes_with_reqwest() {
        let query = SearchIssuesQuery {
            ids: vec![],
            issue_type: Some(IssueType::Bug),
            status: vec![status_key("open")],
            project_id: None,
            assignee: Some(Principal::User {
                name: Username::from("alice"),
            }),
            creator: None,
            q: Some("test query".to_string()),
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
        assert_eq!(
            params.get("assignee").map(String::as_str),
            Some("users/alice")
        );
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
    }

    #[test]
    fn search_issues_query_serializes_empty_query() {
        let query = SearchIssuesQuery::default();

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("labels").map(String::as_str), Some(""));
        assert_eq!(
            params.len(),
            1,
            "only the labels key should exist when no filters are provided"
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
            status: vec![status_key("open"), status_key("in-progress")],
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
            !params.contains_key("status"),
            "empty status vec should be omitted from serialization"
        );
    }

    #[test]
    fn search_issues_query_serializes_status_key_byte_identical_to_legacy() {
        // Wire-contract check (option A): the on-the-wire string for the
        // five legacy enum values must be unchanged after the
        // `Vec<IssueStatus>` -> `Vec<StatusKey>` retype.
        let query = SearchIssuesQuery {
            status: vec![
                status_key("open"),
                status_key("in-progress"),
                status_key("closed"),
                status_key("dropped"),
                status_key("failed"),
            ],
            ..SearchIssuesQuery::default()
        };
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("status").map(String::as_str),
            Some("open,in-progress,closed,dropped,failed")
        );
    }

    #[test]
    fn search_issues_query_deserializes_per_project_status_key() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("status=inbox").unwrap();
        assert_eq!(query.status, vec![status_key("inbox")]);
    }

    #[test]
    fn search_issues_query_deserializes_mixed_legacy_and_project_status_keys() {
        let query: SearchIssuesQuery =
            serde_urlencoded::from_str("status=open%2Cin-progress%2Cinbox").unwrap();
        assert_eq!(
            query.status,
            vec![
                status_key("open"),
                status_key("in-progress"),
                status_key("inbox"),
            ]
        );
    }

    #[test]
    fn search_issues_query_serializes_project_id() {
        let project_id: ProjectId = "j-engr".parse().unwrap();
        let query = SearchIssuesQuery {
            project_id: Some(project_id),
            ..SearchIssuesQuery::default()
        };
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("project_id").map(String::as_str), Some("j-engr"));
    }

    #[test]
    fn search_issues_query_deserializes_project_id() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("project_id=j-engr").unwrap();
        assert_eq!(
            query.project_id.as_ref().map(|p| p.as_ref()),
            Some("j-engr")
        );
    }

    #[test]
    fn search_issues_query_project_id_missing_is_none() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("").unwrap();
        assert!(query.project_id.is_none());
    }

    #[test]
    fn search_issues_query_deserializes_ids() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("ids=i-abcd%2Ci-efgh").unwrap();
        assert_eq!(query.ids.len(), 2);
        assert_eq!(query.ids[0].as_ref(), "i-abcd");
        assert_eq!(query.ids[1].as_ref(), "i-efgh");
    }

    #[test]
    fn search_issues_query_deserializes_assignee_path_form() {
        let query: SearchIssuesQuery =
            serde_urlencoded::from_str("assignee=users%2Falice").unwrap();
        assert_eq!(
            query.assignee,
            Some(Principal::User {
                name: Username::from("alice"),
            })
        );
    }

    #[test]
    fn search_issues_query_assignee_missing_is_none() {
        let query: SearchIssuesQuery = serde_urlencoded::from_str("").unwrap();
        assert_eq!(query.assignee, None);
    }

    #[test]
    fn search_issues_query_assignee_empty_string_is_rejected() {
        // Empty-string and `None` are different values on the wire: an
        // omitted `?assignee=` key parses as `None`, but an explicit
        // `?assignee=` (empty value) should be a parse error rather than
        // silently coerced into `None`.
        let err = serde_urlencoded::from_str::<SearchIssuesQuery>("assignee=")
            .expect_err("empty assignee should fail to deserialize as a Principal");
        assert!(
            err.to_string().to_lowercase().contains("empty"),
            "expected error to mention empty principal: {err}"
        );
    }

    #[test]
    fn search_issues_query_assignee_bare_username_is_rejected() {
        // Bare usernames (without the `users/` prefix) are the legacy v1
        // wire shape. The canonical path form (`users/<x>` / `agents/<x>` /
        // `external/<sys>/<user>`) is required on the URL, so a bare
        // username must fail to parse as a Principal.
        let err = serde_urlencoded::from_str::<SearchIssuesQuery>("assignee=alice")
            .expect_err("bare username should fail to deserialize as a Principal");
        assert!(
            err.to_string().to_lowercase().contains("unknown"),
            "expected error to mention unknown kind: {err}"
        );
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
    fn issue_version_record_serializes_actor_when_present() {
        use crate::actor_ref::{ActorId, ActorRef};

        let issue_id: IssueId = "i-test".parse().unwrap();
        let issue = Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: "test".to_string(),
            creator: Username::from("alice"),
            progress: String::new(),
            status: status_key("open"),
            project_id: None,
            resolved_status: None,
            assignee: None,
            session_settings: Default::default(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
            form: None,
            form_response: None,
            feedback: None,
        };

        let actor = ActorRef::Authenticated {
            actor_id: ActorId::User(Username::from("alice")),
            session_id: None,
        };
        let ts = chrono::Utc::now();
        let record =
            IssueVersionRecord::new(issue_id, 1, ts, issue, Some(actor.clone()), ts, Vec::new());

        let value = serde_json::to_value(&record).expect("should serialize");
        let expected_actor = json!({"Authenticated": {"actor_id": {"User": {"name": "alice"}}}});
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
            status: status_key("open"),
            project_id: None,
            resolved_status: None,
            assignee: None,
            session_settings: Default::default(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
            form: None,
            form_response: None,
            feedback: None,
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

    fn status_key(value: &str) -> StatusKey {
        StatusKey::try_new(value).expect("well-formed status key")
    }

    fn make_test_issue(description: &str) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: description.to_string(),
            creator: Username::from("alice"),
            progress: "some progress text".to_string(),
            status: status_key("in-progress"),
            project_id: None,
            resolved_status: None,
            assignee: Some(Principal::User {
                name: Username::from("bob"),
            }),
            session_settings: SessionSettings {
                repo_name: Some(RepoName::from_str("org/repo").unwrap()),
                ..Default::default()
            },
            dependencies: vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                issue_id("i-parent"),
            )],
            patches: vec!["p-abcd".parse().unwrap()],
            deleted: false,
            form: None,
            form_response: None,
            feedback: None,
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
        assert_eq!(summary.status, status_key("in-progress"));
        assert_eq!(
            summary.assignee,
            Some(Principal::User {
                name: Username::from("bob")
            })
        );
        assert_eq!(summary.dependencies.len(), 1);
        assert_eq!(summary.patches.len(), 1);
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

    mod graph_view {
        use super::*;
        use crate::graph::{GraphView, ObjectKind};

        fn sample_issue() -> Issue {
            Issue {
                issue_type: IssueType::Task,
                title: "Track flakiness".to_string(),
                description: "Investigate flaky CI".to_string(),
                creator: Username::from("alice"),
                progress: "started".to_string(),
                status: super::status_key("in-progress"),
                project_id: None,
                resolved_status: None,
                assignee: Some(Principal::User {
                    name: Username::from("bob"),
                }),
                session_settings: SessionSettings {
                    repo_name: Some(RepoName::from_str("org/repo").unwrap()),
                    ..Default::default()
                },
                dependencies: vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    issue_id("i-parent"),
                )],
                patches: vec!["p-abcd".parse().unwrap()],
                deleted: false,
                form: None,
                form_response: None,
                feedback: Some("look closer".to_string()),
            }
        }

        #[test]
        fn kind_is_issue() {
            assert_eq!(<Issue as GraphView>::KIND, ObjectKind::Issue);
        }

        #[test]
        fn view_l1_matches_expected() {
            let issue = sample_issue();
            assert_eq!(
                issue.view_l1(),
                json!({
                    "title": "Track flakiness",
                    "status": "in-progress",
                })
            );
        }

        #[test]
        fn view_l2_matches_expected() {
            let issue = sample_issue();
            assert_eq!(
                issue.view_l2(),
                json!({
                    "title": "Track flakiness",
                    "status": "in-progress",
                    "assignee": {"User": {"name": "bob"}},
                    "progress": "started",
                    "dependencies": [{
                        "type": "child-of",
                        "issue_id": "i-parent",
                    }],
                })
            );
        }

        #[test]
        fn view_l2_truncates_progress_over_200_chars() {
            let mut issue = sample_issue();
            issue.progress = "a".repeat(250);
            let l2 = issue.view_l2();
            let progress = l2.get("progress").and_then(|v| v.as_str()).unwrap();
            assert_eq!(progress.chars().count(), 203);
            assert!(progress.ends_with("..."));
            assert_eq!(&progress[..200], &"a".repeat(200));
        }

        #[test]
        fn view_l2_does_not_truncate_progress_at_or_under_200_chars() {
            let mut issue = sample_issue();
            issue.progress = "a".repeat(200);
            let l2 = issue.view_l2();
            assert_eq!(
                l2.get("progress").and_then(|v| v.as_str()).unwrap(),
                &"a".repeat(200)
            );

            issue.progress = "short".to_string();
            let l2 = issue.view_l2();
            assert_eq!(
                l2.get("progress").and_then(|v| v.as_str()).unwrap(),
                "short"
            );
        }

        #[test]
        fn view_l2_truncation_is_char_based_for_multibyte() {
            let mut issue = sample_issue();
            // Each emoji is 1 char / 4 bytes — 250 emojis is well over 200 bytes.
            issue.progress = "\u{1F600}".repeat(250);
            let l2 = issue.view_l2();
            let progress = l2.get("progress").and_then(|v| v.as_str()).unwrap();
            assert_eq!(progress.chars().count(), 203);
            assert!(progress.ends_with("..."));
        }

        #[test]
        fn view_l1_does_not_emit_progress() {
            let mut issue = sample_issue();
            issue.progress = "a".repeat(500);
            let l1 = issue.view_l1();
            assert!(l1.get("progress").is_none());
        }

        #[test]
        fn view_l3_preserves_full_progress() {
            let mut issue = sample_issue();
            issue.progress = "a".repeat(500);
            let l3 = issue.view_l3();
            assert_eq!(
                l3.get("progress").and_then(|v| v.as_str()).unwrap().len(),
                500
            );
        }

        #[test]
        fn view_l3_round_trips_to_original() {
            let issue = sample_issue();
            let value = issue.view_l3();
            let roundtripped: Issue = serde_json::from_value(value).unwrap();
            assert_eq!(roundtripped, issue);
        }

        #[test]
        fn view_l2_contains_view_l1_keys_with_same_values() {
            let issue = sample_issue();
            let l1 = issue.view_l1();
            let l2 = issue.view_l2();
            for (key, expected) in l1.as_object().unwrap() {
                assert_eq!(l2.get(key), Some(expected), "key {key} mismatch in L2");
            }
        }
    }
}
