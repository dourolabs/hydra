use super::task_status::{Status, TaskError};
use super::users::Username;
use chrono::{DateTime, Utc};
use hydra_common::api::v1 as api;
use hydra_common::{IssueId, RepoName};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_task_status() -> Status {
    Status::Complete
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub prompt: String,
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
}

impl Session {
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
        status: Status,
        last_message: Option<String>,
        error: Option<TaskError>,
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
            status,
            last_message,
            error,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BundleSpec {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
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

impl From<api::sessions::BundleSpec> for BundleSpec {
    fn from(value: api::sessions::BundleSpec) -> Self {
        match value {
            api::sessions::BundleSpec::None => BundleSpec::None,
            api::sessions::BundleSpec::GitRepository { url, rev } => {
                BundleSpec::GitRepository { url, rev }
            }
            api::sessions::BundleSpec::ServiceRepository { name, rev } => {
                BundleSpec::ServiceRepository { name, rev }
            }
            _ => unreachable!("unsupported bundle spec variant"),
        }
    }
}

impl From<BundleSpec> for api::sessions::BundleSpec {
    fn from(value: BundleSpec) -> Self {
        match value {
            BundleSpec::None => api::sessions::BundleSpec::None,
            BundleSpec::GitRepository { url, rev } => {
                api::sessions::BundleSpec::GitRepository { url, rev }
            }
            BundleSpec::ServiceRepository { name, rev } => {
                api::sessions::BundleSpec::ServiceRepository { name, rev }
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
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
}

impl From<api::sessions::Bundle> for Bundle {
    fn from(value: api::sessions::Bundle) -> Self {
        match value {
            api::sessions::Bundle::None => Bundle::None,
            api::sessions::Bundle::GitRepository { url, rev } => Bundle::GitRepository { url, rev },
            _ => unreachable!("unsupported bundle variant"),
        }
    }
}

impl From<Bundle> for api::sessions::Bundle {
    fn from(value: Bundle) -> Self {
        match value {
            Bundle::None => api::sessions::Bundle::None,
            Bundle::GitRepository { url, rev } => api::sessions::Bundle::GitRepository { url, rev },
        }
    }
}

impl From<api::sessions::Session> for Session {
    fn from(value: api::sessions::Session) -> Self {
        Session {
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
            creation_time: value.creation_time,
            start_time: value.start_time,
            end_time: value.end_time,
        }
    }
}

impl From<Session> for api::sessions::Session {
    fn from(value: Session) -> Self {
        api::sessions::Session::new(
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
            value.creation_time,
            value.start_time,
            value.end_time,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BundleSpec, Session};
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use hydra_common::RepoName;
    use hydra_common::api::v1 as api;
    use std::collections::HashMap;
    use std::str::FromStr;

    #[test]
    fn bundle_spec_converts_between_domain_and_api() {
        let repo = RepoName::from_str("dourolabs/metis").unwrap();
        let domain = BundleSpec::ServiceRepository {
            name: repo.clone(),
            rev: Some("main".to_string()),
        };

        let api_spec: api::sessions::BundleSpec = domain.clone().into();
        let round_trip: BundleSpec = api_spec.into();

        assert_eq!(round_trip, domain);
    }

    #[test]
    fn session_roundtrip_preserves_secrets() {
        let secrets = Some(vec!["db-secret".to_string(), "api-key".to_string()]);
        let domain_session = Session::new(
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
            Status::Created,
            None,
            None,
        );

        let api_session: api::sessions::Session = domain_session.clone().into();
        let round_trip: Session = api_session.into();

        assert_eq!(round_trip.secrets, secrets);
        assert_eq!(round_trip.prompt, domain_session.prompt);
        assert_eq!(round_trip.image, domain_session.image);
        assert_eq!(round_trip.model, domain_session.model);
    }

    #[test]
    fn session_roundtrip_preserves_empty_secrets() {
        let domain_session = Session::new(
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
            Status::Created,
            None,
            None,
        );

        let api_session: api::sessions::Session = domain_session.clone().into();
        let round_trip: Session = api_session.into();

        assert_eq!(round_trip.secrets, None);
    }
}
