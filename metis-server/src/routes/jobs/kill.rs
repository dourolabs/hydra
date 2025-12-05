use crate::{AppState, job_engine::JobEngineError};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::jobs::KillJobResponse;
use tracing::{error, info};

use super::ApiError;

pub async fn kill_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<KillJobResponse>, ApiError> {
    let job_id = job_id.trim().to_string();
    if job_id.is_empty() {
        return Err(ApiError::bad_request("job_id is required"));
    }

    state.job_engine.kill_job(&job_id).await.map_err(|err| match err {
        JobEngineError::NotFound(msg) => {
            error!(job_id = %job_id, error = %msg, "job not found");
            ApiError::not_found(msg)
        }
        JobEngineError::MultipleFound(msg) => {
            error!(job_id = %job_id, error = %msg, "multiple jobs found");
            ApiError::conflict(msg)
        }
        JobEngineError::Kubernetes(kube_err) => {
            error!(job_id = %job_id, error = ?kube_err, "kubernetes error while killing job");
            ApiError::internal(kube_err)
        }
        other => {
            error!(job_id = %job_id, error = %other, "failed to kill job");
            ApiError::internal(other)
        }
    })?;

    info!(job_id = %job_id, "job killed successfully");

    Ok(Json(KillJobResponse {
        job_id,
        status: "killed".to_string(),
    }))
}
