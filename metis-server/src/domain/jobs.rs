use super::task_status::{Status, TaskError, TaskStatusLog};
use metis_common::api::v1 as api;
use metis_common::{IssueId, RepoName, TaskId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_task_status() -> Status {
    Status::Complete
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default = "default_task_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
}

impl Task {
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            image,
            env_vars,
            cpu_limit,
            memory_limit,
            status: Status::Created,
            last_message: None,
            error: None,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
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

impl From<api::jobs::BundleSpec> for BundleSpec {
    fn from(value: api::jobs::BundleSpec) -> Self {
        match value {
            api::jobs::BundleSpec::None => BundleSpec::None,
            api::jobs::BundleSpec::GitRepository { url, rev } => {
                BundleSpec::GitRepository { url, rev }
            }
            api::jobs::BundleSpec::ServiceRepository { name, rev } => {
                BundleSpec::ServiceRepository { name, rev }
            }
            _ => unreachable!("unsupported bundle spec variant"),
        }
    }
}

impl From<BundleSpec> for api::jobs::BundleSpec {
    fn from(value: BundleSpec) -> Self {
        match value {
            BundleSpec::None => api::jobs::BundleSpec::None,
            BundleSpec::GitRepository { url, rev } => {
                api::jobs::BundleSpec::GitRepository { url, rev }
            }
            BundleSpec::ServiceRepository { name, rev } => {
                api::jobs::BundleSpec::ServiceRepository { name, rev }
            }
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

impl From<api::jobs::Bundle> for Bundle {
    fn from(value: api::jobs::Bundle) -> Self {
        match value {
            api::jobs::Bundle::None => Bundle::None,
            api::jobs::Bundle::GitRepository { url, rev } => Bundle::GitRepository { url, rev },
            _ => unreachable!("unsupported bundle variant"),
        }
    }
}

impl From<Bundle> for api::jobs::Bundle {
    fn from(value: Bundle) -> Self {
        match value {
            Bundle::None => api::jobs::Bundle::None,
            Bundle::GitRepository { url, rev } => api::jobs::Bundle::GitRepository { url, rev },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: TaskId,
}

impl CreateJobResponse {
    pub fn new(job_id: TaskId) -> Self {
        Self { job_id }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<JobRecord>,
}

impl ListJobsResponse {
    pub fn new(jobs: Vec<JobRecord>) -> Self {
        Self { jobs }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl KillJobResponse {
    pub fn new(job_id: TaskId, status: String) -> Self {
        Self { job_id, status }
    }
}

impl From<api::jobs::Task> for Task {
    fn from(value: api::jobs::Task) -> Self {
        Task {
            prompt: value.prompt,
            context: value.context.into(),
            spawned_from: value.spawned_from,
            image: value.image,
            env_vars: value.env_vars,
            cpu_limit: value.cpu_limit,
            memory_limit: value.memory_limit,
            status: Status::Created,
            last_message: None,
            error: None,
        }
    }
}

impl From<Task> for api::jobs::Task {
    fn from(value: Task) -> Self {
        api::jobs::Task::new(
            value.prompt,
            value.context.into(),
            value.spawned_from,
            value.image,
            value.env_vars,
            value.cpu_limit,
            value.memory_limit,
        )
    }
}

impl From<Task> for api::jobs::TaskVersion {
    fn from(value: Task) -> Self {
        api::jobs::TaskVersion::new(
            value.prompt,
            value.context.into(),
            value.spawned_from,
            value.image,
            value.env_vars,
            value.cpu_limit,
            value.memory_limit,
            value.status.into(),
            value.last_message,
            value.error.map(Into::into),
        )
    }
}

impl From<api::jobs::CreateJobRequest> for CreateJobRequest {
    fn from(value: api::jobs::CreateJobRequest) -> Self {
        CreateJobRequest {
            prompt: value.prompt,
            image: value.image,
            context: value.context.into(),
            variables: value.variables,
            issue_id: value.issue_id,
        }
    }
}

impl From<CreateJobRequest> for api::jobs::CreateJobRequest {
    fn from(value: CreateJobRequest) -> Self {
        api::jobs::CreateJobRequest::new(
            value.prompt,
            value.image,
            value.context.into(),
            value.variables,
        )
        .with_issue_id(value.issue_id)
    }
}

impl From<api::jobs::CreateJobResponse> for CreateJobResponse {
    fn from(value: api::jobs::CreateJobResponse) -> Self {
        CreateJobResponse {
            job_id: value.job_id,
        }
    }
}

impl From<CreateJobResponse> for api::jobs::CreateJobResponse {
    fn from(value: CreateJobResponse) -> Self {
        api::jobs::CreateJobResponse::new(value.job_id)
    }
}

impl From<api::jobs::JobRecord> for JobRecord {
    fn from(value: api::jobs::JobRecord) -> Self {
        JobRecord {
            id: value.id,
            task: value.task.into(),
            notes: value.notes,
            status_log: value.status_log.into(),
        }
    }
}

impl From<JobRecord> for api::jobs::JobRecord {
    fn from(value: JobRecord) -> Self {
        api::jobs::JobRecord::new(
            value.id,
            value.task.into(),
            value.notes,
            value.status_log.into(),
        )
    }
}

impl From<api::jobs::ListJobsResponse> for ListJobsResponse {
    fn from(value: api::jobs::ListJobsResponse) -> Self {
        ListJobsResponse {
            jobs: value.jobs.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ListJobsResponse> for api::jobs::ListJobsResponse {
    fn from(value: ListJobsResponse) -> Self {
        api::jobs::ListJobsResponse::new(value.jobs.into_iter().map(Into::into).collect())
    }
}

impl From<api::jobs::SearchJobsQuery> for SearchJobsQuery {
    fn from(value: api::jobs::SearchJobsQuery) -> Self {
        SearchJobsQuery {
            q: value.q,
            spawned_from: value.spawned_from,
        }
    }
}

impl From<SearchJobsQuery> for api::jobs::SearchJobsQuery {
    fn from(value: SearchJobsQuery) -> Self {
        api::jobs::SearchJobsQuery::new(value.q, value.spawned_from)
    }
}

impl From<api::jobs::KillJobResponse> for KillJobResponse {
    fn from(value: api::jobs::KillJobResponse) -> Self {
        KillJobResponse {
            job_id: value.job_id,
            status: value.status,
        }
    }
}

impl From<KillJobResponse> for api::jobs::KillJobResponse {
    fn from(value: KillJobResponse) -> Self {
        api::jobs::KillJobResponse::new(value.job_id, value.status)
    }
}

#[cfg(test)]
mod tests {
    use super::{BundleSpec, SearchJobsQuery};
    use metis_common::api::v1 as api;
    use metis_common::{IssueId, RepoName};
    use serde::Serialize;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn serialize_query_params<T: Serialize>(value: &T) -> Vec<(String, String)> {
        let encoded = serde_urlencoded::to_string(value).unwrap();
        serde_urlencoded::from_str(&encoded).unwrap()
    }

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

    #[test]
    fn bundle_spec_converts_between_domain_and_api() {
        let repo = RepoName::from_str("dourolabs/metis").unwrap();
        let domain = BundleSpec::ServiceRepository {
            name: repo.clone(),
            rev: Some("main".to_string()),
        };

        let api_spec: api::jobs::BundleSpec = domain.clone().into();
        let round_trip: BundleSpec = api_spec.into();

        assert_eq!(round_trip, domain);
    }
}
