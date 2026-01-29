#![allow(clippy::too_many_arguments)]

pub mod activity_log;
pub mod api;
pub mod build_cache;
pub mod constants;
pub mod github;
pub mod ids;
pub mod repo_name;
pub mod util;
pub mod versioning;

pub use activity_log::{
    ActivityEvent, ActivityLogEntry, ActivityObjectKind, FieldChange,
    activity_log_for_issue_versions, activity_log_for_job_versions,
    activity_log_for_patch_versions, activity_log_from_versions,
};
pub use api::v1::{
    agents, issues, job_status, jobs, login, logs, merge_queues, patches, repositories,
    task_status, users, whoami,
};
pub use build_cache::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig};
pub use ids::{IssueId, MetisId, MetisIdError, PatchId, TaskId};
pub use repo_name::{RepoName, RepoNameError};
pub use repositories::{
    CreateRepositoryRequest, ListRepositoriesResponse, Repository, RepositoryRecord,
    UpdateRepositoryRequest, UpsertRepositoryResponse,
};
pub use util::EnvGuard;
pub use versioning::{VersionNumber, Versioned};

#[cfg(test)]
pub mod test_helpers {
    use serde::Serialize;

    pub fn serialize_query_params<T: Serialize>(value: &T) -> Vec<(String, String)> {
        let encoded =
            serde_urlencoded::to_string(value).expect("failed to encode query parameters");
        serde_urlencoded::from_str(&encoded)
            .expect("failed to decode encoded query parameters into key/value pairs")
    }
}
