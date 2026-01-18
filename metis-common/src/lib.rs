#![allow(clippy::too_many_arguments)]

pub mod ids;
pub use ids::{IssueId, MetisId, MetisIdError, PatchId, TaskId};

pub mod agents;
pub mod constants;
pub mod issues;
pub mod job_status;
pub mod jobs;
pub mod logs;
pub mod merge_queues;
pub mod patches;
pub mod repo_name;
pub mod repositories;
pub use repo_name::{RepoName, RepoNameError};
pub use repositories::{
    CreateRepositoryRequest, ListRepositoriesResponse, ServiceRepository, ServiceRepositoryConfig,
    ServiceRepositoryInfo, UpdateRepositoryRequest, UpsertRepositoryResponse,
};
pub mod task_status;

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
