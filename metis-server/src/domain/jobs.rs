use super::task_status::{Status, TaskError};
use super::users::Username;
use metis_common::api::v1 as api;
use metis_common::{IssueId, RepoName};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_task_status() -> Status {
    Status::Complete
}

fn default_task_creator() -> Username {
    Username::from("unknown")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub prompt: String,
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    #[serde(default = "default_task_creator")]
    pub creator: Username,
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
    #[serde(default = "default_task_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default)]
    pub deleted: bool,
}

impl Task {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        creator: Username,
        image: Option<String>,
        model: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            creator,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            status: Status::Created,
            last_message: None,
            error: None,
            deleted: false,
        }
    }
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

impl From<api::jobs::Task> for Task {
    fn from(value: api::jobs::Task) -> Self {
        Task {
            prompt: value.prompt,
            context: value.context.into(),
            spawned_from: value.spawned_from,
            creator: value.creator.into(),
            image: value.image,
            model: value.model,
            env_vars: value.env_vars,
            cpu_limit: value.cpu_limit,
            memory_limit: value.memory_limit,
            secrets: value.secrets,
            status: value.status.into(),
            last_message: value.last_message,
            error: value.error.map(Into::into),
            deleted: value.deleted,
        }
    }
}

impl From<Task> for api::jobs::Task {
    fn from(value: Task) -> Self {
        api::jobs::Task::new_with_status(
            value.prompt,
            value.context.into(),
            value.spawned_from,
            value.creator.into(),
            value.image,
            value.model,
            value.env_vars,
            value.cpu_limit,
            value.memory_limit,
            value.secrets,
            value.status.into(),
            value.last_message,
            value.error.map(Into::into),
            value.deleted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BundleSpec, Task};
    use crate::domain::users::Username;
    use metis_common::RepoName;
    use metis_common::api::v1 as api;
    use std::collections::HashMap;
    use std::str::FromStr;

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

    #[test]
    fn task_roundtrip_preserves_secrets() {
        let secrets = Some(vec!["db-secret".to_string(), "api-key".to_string()]);
        let domain_task = Task::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            Some("worker:latest".to_string()),
            Some("gpt-4o".to_string()),
            HashMap::new(),
            Some("400m".to_string()),
            Some("768Mi".to_string()),
            secrets.clone(),
        );

        let api_task: api::jobs::Task = domain_task.clone().into();
        let round_trip: Task = api_task.into();

        assert_eq!(round_trip.secrets, secrets);
        assert_eq!(round_trip.prompt, domain_task.prompt);
        assert_eq!(round_trip.image, domain_task.image);
        assert_eq!(round_trip.model, domain_task.model);
    }

    #[test]
    fn task_roundtrip_preserves_empty_secrets() {
        let domain_task = Task::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );

        let api_task: api::jobs::Task = domain_task.clone().into();
        let round_trip: Task = api_task.into();

        assert_eq!(round_trip.secrets, None);
    }
}
