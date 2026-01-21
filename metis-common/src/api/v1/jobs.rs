use crate::{IssueId, RepoName, TaskId, task_status::TaskStatusLog};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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

impl Task {
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<String>,
        env_vars: HashMap<String, String>,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            image,
            env_vars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateJobRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
}

impl CreateJobRequest {
    pub fn new(
        prompt: String,
        image: Option<String>,
        context: BundleSpec,
        variables: HashMap<String, String>,
    ) -> Self {
        Self {
            prompt,
            image,
            context,
            variables,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
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
        name: RepoName,
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct WorkerContext {
    pub request_context: Bundle,
    pub prompt: String,
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

impl WorkerContext {
    pub fn new(request_context: Bundle, prompt: String, variables: HashMap<String, String>) -> Self {
        Self {
            request_context,
            prompt,
            variables,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateJobResponse {
    pub job_id: TaskId,
}

impl CreateJobResponse {
    pub fn new(job_id: TaskId) -> Self {
        Self { job_id }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListJobsResponse {
    pub jobs: Vec<JobRecord>,
}

impl ListJobsResponse {
    pub fn new(jobs: Vec<JobRecord>) -> Self {
        Self { jobs }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JobRecord {
    pub id: TaskId,
    pub task: Task,
    #[serde(default)]
    pub notes: Option<String>,
    pub status_log: TaskStatusLog,
}

impl JobRecord {
    pub fn new(id: TaskId, task: Task, notes: Option<String>, status_log: TaskStatusLog) -> Self {
        Self {
            id,
            task,
            notes,
            status_log,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchJobsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
}

impl SearchJobsQuery {
    pub fn new(q: Option<String>, spawned_from: Option<IssueId>) -> Self {
        Self { q, spawned_from }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct KillJobResponse {
    pub job_id: TaskId,
    pub status: String,
}

impl KillJobResponse {
    pub fn new(job_id: TaskId, status: String) -> Self {
        Self { job_id, status }
    }
}

#[cfg(test)]
mod tests {
    use super::SearchJobsQuery;
    use crate::{IssueId, test_helpers::serialize_query_params};
    use std::collections::HashMap;

    #[test]
    fn search_jobs_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let query = SearchJobsQuery {
            q: Some("test query".to_string()),
            spawned_from: Some(issue_id.clone()),
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
        assert_eq!(
            params.get("spawned_from").map(String::as_str),
            Some(issue_id.as_ref())
        );
    }

    #[test]
    fn search_jobs_query_serializes_empty_query() {
        let query = SearchJobsQuery::default();

        let params = serialize_query_params(&query);
        assert!(
            params.is_empty(),
            "expected no query params for empty SearchJobsQuery"
        );
    }
}
