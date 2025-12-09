#![allow(clippy::too_many_arguments)]

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
    pub struct TaskStatusLog {
        pub creation_time: DateTime<Utc>,
        #[serde(default)]
        pub start_time: Option<DateTime<Utc>>,
        #[serde(default)]
        pub end_time: Option<DateTime<Utc>>,
        pub current_status: Status,
        #[serde(default)]
        pub failure_reason: Option<String>,
    }
}

pub mod jobs {
    use crate::job_outputs::JobOutputPayload;
    use crate::task_status::TaskStatusLog;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CreateJobRequest {
        pub prompt: String,
        #[serde(default)]
        pub context: CreateJobRequestContext,
        #[serde(default)]
        pub parent_ids: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum CreateJobRequestContext {
        #[serde(rename = "none")]
        None,
        UploadDirectory {
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

    impl Default for CreateJobRequestContext {
        fn default() -> Self {
            Self::None
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct WorkerContext {
        pub request_context: CreateJobRequestContext,
        #[serde(default)]
        pub parents: HashMap<String, JobOutputPayload>,
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
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct JobOutputPayload {
        pub last_message: String,
        pub patch: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JobOutputResponse {
        pub job_id: String,
        pub output: JobOutputPayload,
    }
}
