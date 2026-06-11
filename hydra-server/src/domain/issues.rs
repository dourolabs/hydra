use super::users::Username;
use hydra_common::api::v1 as api;
use hydra_common::api::v1::form::{Form, FormResponse};
use hydra_common::api::v1::projects::StatusKey;
use hydra_common::api::v1::timeout::Timeout;
use hydra_common::principal::Principal;
use hydra_common::{IssueId, PatchId, ProjectId, RepoName};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    #[serde(default)]
    pub title: String,
    pub description: String,
    pub creator: Username,
    #[serde(default)]
    pub progress: String,
    /// Project-scoped status key. Validated against the resolved project's
    /// status list at the route layer (`/v1/issues`); the store does not
    /// reinterpret unknown keys.
    #[serde(default = "default_status_key")]
    pub status: StatusKey,
    /// Project membership. Required on every issue.
    pub project_id: ProjectId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assignee: Option<Principal>,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    #[serde(default)]
    pub dependencies: Vec<IssueDependency>,
    #[serde(default)]
    pub patches: Vec<PatchId>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<Form>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_response: Option<FormResponse>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub feedback: Option<String>,
}

fn default_status_key() -> StatusKey {
    StatusKey::try_new("open").expect("\"open\" is a well-formed StatusKey")
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
        project_id: ProjectId,
        assignee: Option<Principal>,
        session_settings: Option<SessionSettings>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
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
            assignee,
            session_settings: session_settings.unwrap_or_default(),
            dependencies,
            patches,
            deleted: false,
            form,
            form_response,
            feedback,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
    /// Per-session idle timeout for interactive sessions. `None` falls
    /// back to `config.job.interactive_idle_timeout_secs` at handshake
    /// time. `Some(Timeout::Infinite)` means the worker never times out
    /// the conversation due to inactivity.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idle_timeout: Option<Timeout>,
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
        if self.idle_timeout.is_none() {
            self.idle_timeout = other.idle_timeout.take();
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

impl From<api::issues::SessionSettings> for SessionSettings {
    fn from(value: api::issues::SessionSettings) -> Self {
        Self {
            repo_name: value.repo_name,
            remote_url: value.remote_url,
            image: value.image,
            model: value.model,
            branch: value.branch,
            max_retries: value.max_retries,
            cpu_limit: value.cpu_limit,
            memory_limit: value.memory_limit,
            secrets: value.secrets,
            idle_timeout: value.idle_timeout,
        }
    }
}

impl From<SessionSettings> for api::issues::SessionSettings {
    fn from(value: SessionSettings) -> Self {
        let mut session_settings = api::issues::SessionSettings::default();
        session_settings.repo_name = value.repo_name;
        session_settings.remote_url = value.remote_url;
        session_settings.image = value.image;
        session_settings.model = value.model;
        session_settings.branch = value.branch;
        session_settings.max_retries = value.max_retries;
        session_settings.cpu_limit = value.cpu_limit;
        session_settings.memory_limit = value.memory_limit;
        session_settings.secrets = value.secrets;
        session_settings.idle_timeout = value.idle_timeout;
        session_settings
    }
}

impl From<api::issues::Issue> for Issue {
    fn from(value: api::issues::Issue) -> Self {
        // Response-only `status` carries the full `StatusDefinition`;
        // the domain stores only the key, which is the canonical at-rest
        // identifier (project keys are unique within a project). Dropping
        // the resolved fields on the way in preserves the "never stored"
        // invariant so a stale client echo can't shadow the authoritative
        // project definition.
        Self {
            issue_type: value.issue_type.into(),
            title: value.title,
            description: value.description,
            creator: value.creator.into(),
            progress: value.progress,
            status: value.status.key,
            project_id: value.project_id,
            assignee: value.assignee,
            session_settings: value.session_settings.into(),
            dependencies: value.dependencies.into_iter().map(Into::into).collect(),
            patches: value.patches,
            deleted: value.deleted,
            form: value.form,
            form_response: value.form_response,
            feedback: value.feedback,
        }
    }
}

impl From<api::issues::IssueInput> for Issue {
    fn from(value: api::issues::IssueInput) -> Self {
        Self {
            issue_type: value.issue_type.into(),
            title: value.title,
            description: value.description,
            creator: value.creator.into(),
            progress: value.progress,
            status: value.status,
            project_id: value.project_id,
            assignee: value.assignee,
            session_settings: value.session_settings.into(),
            dependencies: value.dependencies.into_iter().map(Into::into).collect(),
            patches: value.patches,
            deleted: value.deleted,
            form: value.form,
            form_response: value.form_response,
            feedback: value.feedback,
        }
    }
}

impl From<Issue> for api::issues::IssueInput {
    fn from(value: Issue) -> Self {
        api::issues::IssueInput::new(
            value.issue_type.into(),
            value.title,
            value.description,
            value.creator.into(),
            value.progress,
            value.status,
            value.project_id,
            value.assignee,
            Some(value.session_settings.into()),
            value.dependencies.into_iter().map(Into::into).collect(),
            value.patches,
            value.deleted,
            value.form,
            value.form_response,
            value.feedback,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::RepoName;
    use hydra_common::api::v1 as api;
    use std::str::FromStr;

    #[test]
    fn session_settings_roundtrip_preserves_secrets() {
        let secrets = Some(vec!["db-secret".to_string(), "api-key".to_string()]);
        let session_settings = SessionSettings {
            repo_name: Some(RepoName::from_str("dourolabs/hydra").unwrap()),
            remote_url: None,
            image: Some("worker:latest".to_string()),
            model: None,
            branch: None,
            max_retries: None,
            cpu_limit: None,
            memory_limit: None,
            secrets: secrets.clone(),
            idle_timeout: None,
        };

        let api_session_settings: api::issues::SessionSettings = session_settings.clone().into();
        let round_trip: SessionSettings = api_session_settings.into();

        assert_eq!(round_trip.secrets, secrets);
        assert_eq!(round_trip.repo_name, session_settings.repo_name);
        assert_eq!(round_trip.image, session_settings.image);
    }

    #[test]
    fn session_settings_merge_prefers_primary_secrets() {
        let primary = SessionSettings {
            repo_name: None,
            remote_url: None,
            image: None,
            model: None,
            branch: None,
            max_retries: None,
            cpu_limit: None,
            memory_limit: None,
            secrets: Some(vec!["primary-secret".to_string()]),
            idle_timeout: None,
        };
        let secondary = SessionSettings {
            repo_name: None,
            remote_url: None,
            image: None,
            model: None,
            branch: None,
            max_retries: None,
            cpu_limit: None,
            memory_limit: None,
            secrets: Some(vec!["secondary-secret".to_string()]),
            idle_timeout: None,
        };

        let merged = SessionSettings::merge(primary, secondary);

        assert_eq!(merged.secrets, Some(vec!["primary-secret".to_string()]));
    }

    #[test]
    fn session_settings_merge_uses_secondary_when_primary_none() {
        let primary = SessionSettings {
            repo_name: None,
            remote_url: None,
            image: None,
            model: None,
            branch: None,
            max_retries: None,
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            idle_timeout: None,
        };
        let secondary = SessionSettings {
            repo_name: None,
            remote_url: None,
            image: None,
            model: None,
            branch: None,
            max_retries: None,
            cpu_limit: None,
            memory_limit: None,
            secrets: Some(vec!["secondary-secret".to_string()]),
            idle_timeout: None,
        };

        let merged = SessionSettings::merge(primary, secondary);

        assert_eq!(merged.secrets, Some(vec!["secondary-secret".to_string()]));
    }

    /// `SessionSettings::merge` follows the same primary-wins / fill-from-
    /// secondary contract for `idle_timeout` as it does for every other
    /// field — the merge logic itself is None-fill, but covering this
    /// explicitly anchors the issue > status > default precedence the
    /// merge layer is wired into.
    #[test]
    fn session_settings_merge_prefers_primary_idle_timeout() {
        let infinite = Some(Timeout::Infinite);
        let sixty = Timeout::seconds(60);
        let primary = SessionSettings {
            idle_timeout: infinite,
            ..Default::default()
        };
        let secondary = SessionSettings {
            idle_timeout: sixty,
            ..Default::default()
        };
        let merged = SessionSettings::merge(primary, secondary);
        assert_eq!(merged.idle_timeout, infinite);
    }

    #[test]
    fn session_settings_merge_fills_idle_timeout_from_secondary() {
        let primary = SessionSettings::default();
        let sixty = Timeout::seconds(60);
        let secondary = SessionSettings {
            idle_timeout: sixty,
            ..Default::default()
        };
        let merged = SessionSettings::merge(primary, secondary);
        assert_eq!(merged.idle_timeout, sixty);
    }

    /// `SessionSettings.idle_timeout` round-trips through serde — both
    /// `Some(Timeout::Infinite)` and `Some(Timeout::Seconds(_))` survive
    /// the issue's `job_settings_json` storage shape.
    #[test]
    fn session_settings_idle_timeout_round_trips_through_json() {
        for value in [
            Some(Timeout::Infinite),
            Timeout::seconds(600),
            Timeout::seconds(1),
            None,
        ] {
            let settings = SessionSettings {
                idle_timeout: value,
                ..Default::default()
            };
            let json = serde_json::to_value(&settings).unwrap();
            let parsed: SessionSettings = serde_json::from_value(json).unwrap();
            assert_eq!(parsed.idle_timeout, value);
        }
    }
}
