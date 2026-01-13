use crate::TaskId;
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
    pub job_id: TaskId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: TaskId,
    #[serde(default)]
    pub notes: Option<String>,
    pub program: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    pub status_log: TaskStatusLog,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KillJobResponse {
    pub job_id: TaskId,
    pub status: String,
}
