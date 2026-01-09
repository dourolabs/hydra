#![allow(clippy::too_many_arguments)]

pub mod constants;
pub mod artifacts {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", content = "value", rename_all = "snake_case")]
    pub enum Artifact {
        Patch { diff: String },
        Issue { description: String },
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

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UpsertArtifactRequest {
        pub artifact: Artifact,
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
        pub q: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ListArtifactsResponse {
        pub artifacts: Vec<ArtifactRecord>,
    }
}
pub mod task_status {
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
            artifact_ids: Vec<String>,
        },
        Completed {
            at: DateTime<Utc>,
        },
        Failed {
            at: DateTime<Utc>,
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
                Event::Completed { at } | Event::Failed { at } => Some(*at),
                _ => None,
            })
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
    }
}

pub mod jobs {
    use crate::job_outputs::JobOutputPayload;
    use crate::task_status::TaskStatusLog;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ParentContext {
        #[serde(default)]
        pub name: Option<String>,
        pub output: JobOutputPayload,
    }

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
        pub parent_ids: Vec<String>,
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
        #[serde(default)]
        pub parents: HashMap<String, ParentContext>,
        pub program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub params: Vec<String>,
        #[serde(default)]
        pub variables: HashMap<String, String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct CreateJobResponse {
        pub job_id: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ListJobsResponse {
        pub jobs: Vec<JobSummary>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct JobSummary {
        pub id: String,
        #[serde(default)]
        pub notes: Option<String>,
        pub program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub params: Vec<String>,
        pub status_log: TaskStatusLog,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct KillJobResponse {
        pub job_id: String,
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

pub mod job_outputs {
    use crate::jobs::Bundle;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct JobOutputPayload {
        pub last_message: String,
        pub patch: String,
        pub bundle: Bundle,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JobOutputResponse {
        pub job_id: String,
        pub output: JobOutputPayload,
    }
}
