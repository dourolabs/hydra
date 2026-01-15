use crate::task_status::TaskStatusLog;
use crate::{IssueId, TaskId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub prompt: String,
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub prompt: String,
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
    GitRepository {
        /// Remote Git repository URL that should be cloned for the job context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
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
            Bundle::GitRepository { url, rev } => BundleSpec::GitRepository { url, rev },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Bundle {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the job context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerContext {
    pub request_context: Bundle,
    pub prompt: String,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_repo_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: TaskId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<JobRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: TaskId,
    pub task: Task,
    #[serde(default)]
    pub notes: Option<String>,
    pub status_log: TaskStatusLog,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchJobsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KillJobResponse {
    pub job_id: TaskId,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::SearchJobsQuery;
    use crate::IssueId;
    use std::collections::HashMap;

    #[test]
    fn search_jobs_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let query = SearchJobsQuery {
            q: Some("test query".to_string()),
            spawned_from: Some(issue_id.clone()),
        };

        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/jobs")
            .query(&query)
            .build()
            .map(|request| {
                request
                    .url()
                    .query_pairs()
                    .into_owned()
                    .collect::<HashMap<_, _>>()
            });

        let params = result.expect("Failed to serialize SearchJobsQuery with reqwest");
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
        assert_eq!(
            params.get("spawned_from").map(String::as_str),
            Some(issue_id.as_ref())
        );
    }

    #[test]
    fn search_jobs_query_serializes_empty_query() {
        let query = SearchJobsQuery::default();

        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/jobs")
            .query(&query)
            .build();

        result.expect("Failed to serialize empty SearchJobsQuery");
    }
}
