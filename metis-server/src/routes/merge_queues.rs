use crate::{
    app::{AppState, MergeQueueError},
    routes::jobs::ApiError,
};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::{
    RepoName,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
};
use tracing::{error, info};

pub async fn get_merge_queue(
    State(state): State<AppState>,
    Path((organization, repo, branch_name)): Path<(String, String, String)>,
) -> Result<Json<MergeQueue>, ApiError> {
    let repo_name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(
        service_repo = %repo_name,
        branch = %branch_name,
        "get_merge_queue invoked"
    );
    let queue = state
        .merge_queue(&repo_name, &branch_name)
        .await
        .map_err(map_merge_queue_error)?;

    Ok(Json(queue))
}

pub async fn enqueue_patch(
    State(state): State<AppState>,
    Path((organization, repo, branch_name)): Path<(String, String, String)>,
    Json(request): Json<EnqueueMergePatchRequest>,
) -> Result<Json<MergeQueue>, ApiError> {
    let repo_name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(
        service_repo = %repo_name,
        branch = %branch_name,
        patch_id = %request.patch_id,
        "enqueue_merge_patch invoked"
    );

    let queue = state
        .enqueue_merge_queue_patch(&repo_name, &branch_name, request.patch_id)
        .await
        .map_err(map_merge_queue_error)?;

    Ok(Json(queue))
}

fn map_merge_queue_error(err: MergeQueueError) -> ApiError {
    match err {
        MergeQueueError::UnknownRepository(name) => {
            error!(service_repo = %name, "unknown repository for merge queue");
            ApiError::bad_request(format!("unknown repository '{name}'"))
        }
    }
}
