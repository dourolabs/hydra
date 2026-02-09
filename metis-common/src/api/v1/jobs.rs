use crate::{
    BuildCacheContext, IssueId, RepoName, TaskId, VersionNumber,
    task_status::{Status, TaskError, TaskStatusLog},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(default = "default_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Task {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<String>,
        model: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
        deleted: bool,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            status: Status::Created,
            last_message: None,
            error: None,
            deleted,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_status(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<String>,
        model: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
        status: Status,
        last_message: Option<String>,
        error: Option<TaskError>,
        deleted: bool,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            status,
            last_message,
            error,
            deleted,
        }
    }
}

fn default_status() -> Status {
    Status::Created
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
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
            issue_id: None,
        }
    }

    pub fn with_issue_id(mut self, issue_id: Option<IssueId>) -> Self {
        self.issue_id = issue_id;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    #[serde(other)]
    Unknown,
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
            Bundle::Unknown => BundleSpec::Unknown,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleSpecHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
    ServiceRepository {
        name: RepoName,
        #[serde(default)]
        rev: Option<String>,
    },
}

impl<'de> Deserialize<'de> for BundleSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleSpecHelper>(value) {
            Ok(BundleSpecHelper::None) => Ok(BundleSpec::None),
            Ok(BundleSpecHelper::GitRepository { url, rev }) => {
                Ok(BundleSpec::GitRepository { url, rev })
            }
            Ok(BundleSpecHelper::ServiceRepository { name, rev }) => {
                Ok(BundleSpec::ServiceRepository { name, rev })
            }
            Err(_) => Ok(BundleSpec::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
}

impl<'de> Deserialize<'de> for Bundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleHelper>(value) {
            Ok(BundleHelper::None) => Ok(Bundle::None),
            Ok(BundleHelper::GitRepository { url, rev }) => Ok(Bundle::GitRepository { url, rev }),
            Err(_) => Ok(Bundle::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkerContext {
    pub request_context: Bundle,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache: Option<BuildCacheContext>,
}

impl WorkerContext {
    pub fn new(
        request_context: Bundle,
        prompt: String,
        model: Option<String>,
        variables: HashMap<String, String>,
        build_cache: Option<BuildCacheContext>,
    ) -> Self {
        Self {
            request_context,
            prompt,
            model,
            variables,
            build_cache,
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

    /// Clears large fields that are unnecessary for list responses.
    ///
    /// Specifically: truncates `task.prompt` to the first 100 characters,
    /// sets `task.last_message` to `None`, and clears `last_message` from
    /// any `Completed` events in the status log.
    pub fn strip_large_fields(&mut self) {
        // Truncate prompt to first 100 chars
        if self.task.prompt.len() > 100 {
            let truncated: String = self.task.prompt.chars().take(100).collect();
            self.task.prompt = truncated;
        }

        // Clear last_message from task
        self.task.last_message = None;

        // Clear last_message from Completed events in the status log
        for event in &mut self.status_log.events {
            if let crate::task_status::Event::Completed { last_message, .. } = event {
                *last_message = None;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct JobVersionRecord {
    pub job_id: TaskId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub task: Task,
}

impl JobVersionRecord {
    pub fn new(
        job_id: TaskId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        task: Task,
    ) -> Self {
        Self {
            job_id,
            version,
            timestamp,
            task,
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
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListJobVersionsResponse {
    pub versions: Vec<JobVersionRecord>,
}

impl ListJobVersionsResponse {
    pub fn new(versions: Vec<JobVersionRecord>) -> Self {
        Self { versions }
    }
}

impl SearchJobsQuery {
    pub fn new(
        q: Option<String>,
        spawned_from: Option<IssueId>,
        include_deleted: Option<bool>,
    ) -> Self {
        Self {
            q,
            spawned_from,
            include_deleted,
        }
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
    use super::*;
    use crate::task_status::{Event, Status, TaskStatusLog};
    use crate::{IssueId, test_helpers::serialize_query_params};
    use chrono::Utc;
    use std::collections::HashMap;

    #[test]
    fn search_jobs_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let query = SearchJobsQuery {
            q: Some("test query".to_string()),
            spawned_from: Some(issue_id.clone()),
            include_deleted: None,
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

    #[test]
    fn strip_large_fields_clears_prompt_last_message_and_completed_events() {
        let now = Utc::now();
        let long_prompt = "x".repeat(500);
        let task = Task::new_with_status(
            long_prompt,
            BundleSpec::None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Complete,
            Some("very large output".to_string()),
            None,
            false,
        );

        let status_log = TaskStatusLog::from_events(vec![
            Event::Created {
                at: now,
                status: Status::Created,
            },
            Event::Started { at: now },
            Event::Completed {
                at: now,
                last_message: Some("large completion message".to_string()),
            },
        ]);

        let task_id = crate::TaskId::new();
        let mut record = JobRecord::new(task_id, task, Some("note".to_string()), status_log);

        record.strip_large_fields();

        // Prompt should be truncated to 100 chars
        assert_eq!(record.task.prompt.len(), 100);
        assert!(record.task.prompt.chars().all(|c| c == 'x'));

        // last_message on task should be cleared
        assert_eq!(record.task.last_message, None);

        // last_message on Completed event should be cleared
        for event in &record.status_log.events {
            if let Event::Completed { last_message, .. } = event {
                assert_eq!(*last_message, None);
            }
        }

        // Notes should be preserved
        assert_eq!(record.notes, Some("note".to_string()));
    }

    #[test]
    fn strip_large_fields_preserves_short_prompt() {
        let now = Utc::now();
        let short_prompt = "short prompt".to_string();
        let task = Task::new_with_status(
            short_prompt.clone(),
            BundleSpec::None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
            false,
        );

        let status_log = TaskStatusLog::from_events(vec![Event::Created {
            at: now,
            status: Status::Created,
        }]);

        let task_id = crate::TaskId::new();
        let mut record = JobRecord::new(task_id, task, None, status_log);

        record.strip_large_fields();

        // Short prompt should be preserved as-is
        assert_eq!(record.task.prompt, short_prompt);
    }
}
