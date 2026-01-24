use crate::app::{AppState, MergeQueueError};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::{
    RepoName,
    api::v1::{
        ApiError,
        merge_queues::{EnqueueMergePatchRequest, MergeQueue},
    },
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

    info!(
        service_repo = %repo_name,
        branch = %branch_name,
        queue_len = queue.patches.len(),
        "get_merge_queue completed"
    );
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

    info!(
        service_repo = %repo_name,
        branch = %branch_name,
        queue_len = queue.patches.len(),
        "enqueue_merge_patch completed"
    );
    Ok(Json(queue))
}

fn map_merge_queue_error(err: MergeQueueError) -> ApiError {
    match err {
        MergeQueueError::UnknownRepository(name) => {
            error!(service_repo = %name, "unknown repository for merge queue");
            ApiError::bad_request(format!("unknown repository '{name}'"))
        }
        MergeQueueError::PatchNotFound { patch_id } => {
            error!(patch_id = %patch_id, "patch not found when enqueuing merge request");
            ApiError::not_found(format!("patch '{patch_id}' not found"))
        }
        MergeQueueError::PatchRepositoryMismatch {
            patch_id,
            patch_repo,
            service_repo,
        } => {
            error!(
                patch_id = %patch_id,
                patch_repo = %patch_repo,
                service_repo = %service_repo,
                "patch targets different repository"
            );
            ApiError::bad_request(format!(
                "patch '{patch_id}' targets repository '{patch_repo}', not '{service_repo}'"
            ))
        }
        MergeQueueError::PatchLookup { patch_id, source } => {
            error!(patch_id = %patch_id, error = %source, "failed to load patch");
            ApiError::internal(anyhow::anyhow!("failed to load patch '{patch_id}'"))
        }
        MergeQueueError::RepositoryLookup { repo_name, source } => {
            error!(
                service_repo = %repo_name,
                error = %source,
                "failed to load repository for merge queue"
            );
            ApiError::internal(anyhow::anyhow!(
                "failed to load repository '{repo_name}' for merge queue"
            ))
        }
        MergeQueueError::Git { repo_name, source } => {
            error!(service_repo = %repo_name, error = %source, "git error while updating merge queue");
            ApiError::internal(anyhow::anyhow!(
                "git error while updating merge queue for '{repo_name}'"
            ))
        }
        MergeQueueError::QueueInitialization {
            repo_name,
            branch_name,
            source,
        } => {
            error!(
                service_repo = %repo_name,
                branch = %branch_name,
                error = %source,
                "failed to initialize merge queue"
            );
            ApiError::internal(anyhow::anyhow!(
                "failed to initialize merge queue for '{repo_name}:{branch_name}'"
            ))
        }
        MergeQueueError::QueueUpdate {
            patch_id,
            repo_name,
            branch_name,
            source,
        } => {
            error!(
                patch_id = %patch_id,
                service_repo = %repo_name,
                branch = %branch_name,
                error = %source,
                "failed to append patch to merge queue"
            );
            ApiError::internal(anyhow::anyhow!(
                "failed to append patch '{patch_id}' to merge queue for '{repo_name}:{branch_name}'"
            ))
        }
    }
}
