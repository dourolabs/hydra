use crate::{routes::jobs::ApiError, store::StoreError};
use anyhow::anyhow;
use tracing::error;

pub mod agents;
pub mod issues;
pub mod jobs;
pub mod merge_queues;
pub mod patches;
pub mod repositories;
pub mod users;

pub(crate) fn map_emit_error(err: StoreError, job_id: &str) -> ApiError {
    match err {
        StoreError::TaskNotFound(id) => {
            error!(job_id = %id, "job not found when emitting artifacts");
            ApiError::not_found(format!("job '{id}' not found"))
        }
        StoreError::InvalidStatusTransition => {
            error!(job_id = %job_id, "job not running when emitting artifacts");
            ApiError::bad_request("job must be running to record emitted artifacts")
        }
        other => {
            error!(job_id = %job_id, error = %other, "failed to emit artifacts");
            ApiError::internal(anyhow!("failed to emit artifacts for '{job_id}': {other}"))
        }
    }
}
