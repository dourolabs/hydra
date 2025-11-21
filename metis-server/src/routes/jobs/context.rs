use crate::{AppState, routes::jobs::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::jobs::CreateJobRequestContext;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<CreateJobRequestContext>, ApiError> {
    let job_id = job_id.trim();
    info!(job_id = %job_id, "get_job_context invoked");
    if job_id.is_empty() {
        error!("get_job_context received an empty job_id");
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    let store = state.job_contexts.read().await;
    match store.get(job_id) {
        Some(ctx) => Ok(Json(ctx.clone())),
        None => {
            error!(job_id = %job_id, "context not found for job_id");
            Err(ApiError::not_found("context not found"))
        }
    }
}

