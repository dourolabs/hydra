#![allow(clippy::too_many_arguments)]

/// Identifier used for jobs, tasks, and artifacts within Metis.
pub type MetisId = String;

pub mod constants;
pub mod artifacts {
    use crate::MetisId;
    use serde::{Deserialize, Serialize};
    use std::{fmt, str::FromStr};

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum IssueStatus {
        #[default]
        Open,
        InProgress,
        Closed,
    }

    impl IssueStatus {
        pub fn as_str(&self) -> &'static str {
            match self {
                IssueStatus::Open => "open",
                IssueStatus::InProgress => "in-progress",
                IssueStatus::Closed => "closed",
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

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        pub issue_id: MetisId,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", content = "value", rename_all = "snake_case")]
    pub enum Artifact {
        Patch {
            diff: String,
            description: String,
        },
        Issue {
            #[serde(rename = "type")]
            issue_type: IssueType,
            description: String,
            #[serde(default)]
            status: IssueStatus,
            #[serde(skip_serializing_if = "Option::is_none", default)]
            assignee: Option<String>,
            #[serde(default)]
            dependencies: Vec<IssueDependency>,
        },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ArtifactKind {
        Patch,
        Issue,
    }

    impl From<&Artifact> for ArtifactKind {
        fn from(artifact: &Artifact) -> Self {
            match artifact {
                Artifact::Patch { .. } => ArtifactKind::Patch,
                Artifact::Issue { .. } => ArtifactKind::Issue,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ArtifactRecord {
        pub id: String,
        pub artifact: Artifact,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct UpsertArtifactRequest {
        pub artifact: Artifact,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub job_id: Option<MetisId>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct UpsertArtifactResponse {
        pub artifact_id: String,
    }

    #[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SearchArtifactsQuery {
        #[serde(default, rename = "type")]
        pub artifact_type: Option<ArtifactKind>,
        #[serde(default)]
        pub issue_type: Option<IssueType>,
        #[serde(default)]
        pub status: Option<IssueStatus>,
        #[serde(default)]
        pub q: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ListArtifactsResponse {
        pub artifacts: Vec<ArtifactRecord>,
    }
}
pub mod task_status {
    use crate::MetisId;
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Status {
        Blocked,
        Pending,
        Running,
        Complete,
        Failed,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TaskError {
        JobEngineError { reason: String },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Event {
        Created {
            at: DateTime<Utc>,
            status: Status,
        },
        Started {
            at: DateTime<Utc>,
        },
        Emitted {
            at: DateTime<Utc>,
            /// MetisIds for any artifacts produced by the task at this moment.
            artifact_ids: Vec<MetisId>,
        },
        Completed {
            at: DateTime<Utc>,
        },
        Failed {
            at: DateTime<Utc>,
            error: TaskError,
        },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TaskStatusLog {
        #[serde(default)]
        pub events: Vec<Event>,
    }

    impl TaskStatusLog {
        pub fn new(initial_status: Status, at: DateTime<Utc>) -> Self {
            Self {
                events: vec![Event::Created {
                    at,
                    status: initial_status,
                }],
            }
        }

        pub fn current_status(&self) -> Status {
            self.events
                .iter()
                .rev()
                .find_map(|event| match event {
                    Event::Created { status, .. } => Some(*status),
                    Event::Started { .. } => Some(Status::Running),
                    Event::Completed { .. } => Some(Status::Complete),
                    Event::Failed { .. } => Some(Status::Failed),
                    Event::Emitted { .. } => None,
                })
                .unwrap_or(Status::Pending)
        }

        pub fn creation_time(&self) -> Option<DateTime<Utc>> {
            self.events.iter().find_map(|event| match event {
                Event::Created { at, .. } => Some(*at),
                _ => None,
            })
        }

        pub fn start_time(&self) -> Option<DateTime<Utc>> {
            self.events.iter().find_map(|event| match event {
                Event::Started { at } => Some(*at),
                _ => None,
            })
        }

        pub fn end_time(&self) -> Option<DateTime<Utc>> {
            self.events.iter().rev().find_map(|event| match event {
                Event::Completed { at, .. } | Event::Failed { at, .. } => Some(*at),
                _ => None,
            })
        }

        pub fn result(&self) -> Option<Result<(), TaskError>> {
            self.events.iter().rev().find_map(|event| match event {
                Event::Completed { .. } => Some(Ok(())),
                Event::Failed { error, .. } => Some(Err(error.clone())),
                _ => None,
            })
        }

        pub fn emitted_artifacts(&self) -> Option<Vec<MetisId>> {
            let mut artifact_ids = Vec::new();

            for event in &self.events {
                if let Event::Emitted {
                    artifact_ids: ids, ..
                } = event
                {
                    artifact_ids.extend(ids.clone());
                }
            }

            if artifact_ids.is_empty() {
                None
            } else {
                Some(artifact_ids)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use chrono::Utc;

        #[test]
        fn current_status_ignores_emitted_events() {
            let now = Utc::now();
            let mut log = TaskStatusLog::new(Status::Pending, now);
            log.events.push(Event::Started { at: now });
            log.events.push(Event::Emitted {
                at: now,
                artifact_ids: vec!["artifact-1".into(), "artifact-2".into()],
            });

            assert_eq!(log.current_status(), Status::Running);
        }

        #[test]
        fn emitted_artifacts_returns_none_when_missing() {
            let now = Utc::now();
            let log = TaskStatusLog::new(Status::Pending, now);

            assert_eq!(log.emitted_artifacts(), None);
        }

        #[test]
        fn emitted_artifacts_collects_all_in_order() {
            let now = Utc::now();
            let mut log = TaskStatusLog::new(Status::Pending, now);
            log.events.push(Event::Started { at: now });
            log.events.push(Event::Emitted {
                at: now,
                artifact_ids: vec!["artifact-1".into()],
            });
            log.events.push(Event::Emitted {
                at: now,
                artifact_ids: vec!["artifact-2".into(), "artifact-3".into()],
            });

            assert_eq!(
                log.emitted_artifacts(),
                Some(vec![
                    "artifact-1".into(),
                    "artifact-2".into(),
                    "artifact-3".into()
                ])
            );
        }
    }
}

pub mod jobs {
    use crate::MetisId;
    use crate::task_status::TaskStatusLog;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CreateJobRequest {
        pub program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub params: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub image: Option<String>,
        #[serde(default)]
        pub context: BundleSpec,
        #[serde(default)]
        pub parent_ids: Vec<MetisId>,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        pub variables: HashMap<String, String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum BundleSpec {
        #[serde(rename = "none")]
        None,
        TarGz {
            /// Base64-encoded archive (.tar.gz) of the directory contents.
            archive_base64: String,
        },
        GitRepository {
            /// Remote Git repository URL that should be cloned for the job context.
            url: String,
            /// Specific git revision (branch, tag, or commit) to checkout after cloning.
            rev: String,
        },
        GitBundle {
            /// Base64-encoded git bundle representing the repository HEAD.
            bundle_base64: String,
        },
        ServiceRepository {
            /// Name of a repository configured in the service configuration.
            name: String,
            /// Optional git revision (branch, tag, or commit) to checkout after cloning.
            #[serde(default)]
            rev: Option<String>,
        },
    }

    impl Default for BundleSpec {
        fn default() -> Self {
            Self::None
        }
    }

    impl From<Bundle> for BundleSpec {
        fn from(bundle: Bundle) -> Self {
            match bundle {
                Bundle::None => BundleSpec::None,
                Bundle::TarGz { archive_base64 } => BundleSpec::TarGz { archive_base64 },
                Bundle::GitRepository { url, rev } => BundleSpec::GitRepository { url, rev },
                Bundle::GitBundle { bundle_base64 } => BundleSpec::GitBundle { bundle_base64 },
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum Bundle {
        #[serde(rename = "none")]
        None,
        TarGz {
            /// Base64-encoded archive (.tar.gz) of the directory contents.
            archive_base64: String,
        },
        GitRepository {
            /// Remote Git repository URL that should be cloned for the job context.
            url: String,
            /// Specific git revision (branch, tag, or commit) to checkout after cloning.
            rev: String,
        },
        GitBundle {
            /// Base64-encoded git bundle representing the repository HEAD.
            bundle_base64: String,
        },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct WorkerContext {
        pub request_context: Bundle,
        pub program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub params: Vec<String>,
        #[serde(default)]
        pub variables: HashMap<String, String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct CreateJobResponse {
        pub job_id: MetisId,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ListJobsResponse {
        pub jobs: Vec<JobSummary>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct JobSummary {
        pub id: MetisId,
        #[serde(default)]
        pub notes: Option<String>,
        pub program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub params: Vec<String>,
        pub status_log: TaskStatusLog,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct KillJobResponse {
        pub job_id: MetisId,
        pub status: String,
    }
}

pub mod logs {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub struct LogsQuery {
        #[serde(default)]
        pub watch: Option<bool>,
        #[serde(default)]
        pub tail_lines: Option<i64>,
    }
}

pub mod job_status {
    use crate::{
        MetisId,
        task_status::{Status, TaskError, TaskStatusLog},
    };
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    pub enum JobStatusUpdate {
        Complete,
        Failed { reason: String },
    }

    impl JobStatusUpdate {
        pub fn to_result(&self) -> Result<(), TaskError> {
            match self {
                JobStatusUpdate::Complete => Ok(()),
                JobStatusUpdate::Failed { reason } => Err(TaskError::JobEngineError {
                    reason: reason.clone(),
                }),
            }
        }

        pub fn as_status(&self) -> Status {
            match self {
                JobStatusUpdate::Complete => Status::Complete,
                JobStatusUpdate::Failed { .. } => Status::Failed,
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SetJobStatusResponse {
        pub job_id: MetisId,
        pub status: Status,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GetJobStatusResponse {
        pub job_id: MetisId,
        pub status_log: TaskStatusLog,
    }
}
