use crate::{
    app::{AppState, UpsertPatchError},
    routes::jobs::ApiError,
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{
    PatchId,
    patches::{
        ListPatchesResponse, Patch, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct PatchIdPath(pub PatchId);

#[async_trait]
impl<S> FromRequestParts<S> for PatchIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(patch_id) = Path::<PatchId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(patch_id))
    }
}

pub async fn create_patch(
    State(state): State<AppState>,
    Json(payload): Json<UpsertPatchRequest>,
) -> Result<Json<UpsertPatchResponse>, ApiError> {
    info!("create_patch invoked");
    let patch_id = state
        .upsert_patch(None, payload)
        .await
        .map_err(map_upsert_patch_error)?;

    Ok(Json(UpsertPatchResponse { patch_id }))
}

pub async fn update_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
    Json(payload): Json<UpsertPatchRequest>,
) -> Result<Json<UpsertPatchResponse>, ApiError> {
    info!(patch_id = %patch_id, "update_patch invoked");
    let patch_id = state
        .upsert_patch(Some(patch_id), payload)
        .await
        .map_err(map_upsert_patch_error)?;

    Ok(Json(UpsertPatchResponse { patch_id }))
}

pub async fn get_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<PatchRecord>, ApiError> {
    info!(patch_id = %patch_id, "get_patch invoked");
    let store_read = state.store.read().await;
    let patch = store_read
        .get_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    Ok(Json(PatchRecord {
        id: patch_id,
        patch,
    }))
}

pub async fn list_patches(
    State(state): State<AppState>,
    Query(query): Query<SearchPatchesQuery>,
) -> Result<Json<ListPatchesResponse>, ApiError> {
    info!(query = ?query.q, "list_patches invoked");

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let patches = store_read
        .list_patches()
        .await
        .map_err(|err| map_patch_error(err, None))?;

    let filtered = patches
        .into_iter()
        .filter(|(id, patch)| patch_matches(search_term.as_deref(), id, patch))
        .map(|(id, patch)| PatchRecord { id, patch })
        .collect();

    Ok(Json(ListPatchesResponse { patches: filtered }))
}

fn patch_matches(search_term: Option<&str>, patch_id: &PatchId, patch: &Patch) -> bool {
    if let Some(term) = search_term {
        let lower_id = patch_id.to_string().to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return patch.title.to_lowercase().contains(term)
            || patch.description.to_lowercase().contains(term)
            || format!("{:?}", patch.status).to_lowercase().contains(term)
            || patch
                .commit_range
                .base
                .to_string()
                .to_lowercase()
                .contains(term)
            || patch
                .commit_range
                .head
                .to_string()
                .to_lowercase()
                .contains(term)
            || patch
                .service_repo_name
                .to_string()
                .to_lowercase()
                .contains(term)
            || patch
                .github
                .as_ref()
                .map(|github| {
                    github.owner.to_lowercase().contains(term)
                        || github.repo.to_lowercase().contains(term)
                        || github.number.to_string().contains(term)
                        || github
                            .head_ref
                            .as_deref()
                            .map(|value| value.to_lowercase().contains(term))
                            .unwrap_or(false)
                        || github
                            .base_ref
                            .as_deref()
                            .map(|value| value.to_lowercase().contains(term))
                            .unwrap_or(false)
                })
                .unwrap_or(false);
    }

    true
}

fn map_upsert_patch_error(err: UpsertPatchError) -> ApiError {
    match err {
        UpsertPatchError::JobIdProvidedForUpdate => {
            ApiError::bad_request("job_id may only be provided when creating a patch")
        }
        UpsertPatchError::JobNotFound { job_id, .. } => {
            error!(job_id = %job_id, "job not found when creating patch");
            ApiError::not_found(format!("job '{job_id}' not found"))
        }
        UpsertPatchError::JobStatusLookup { job_id, source } => {
            error!(job_id = %job_id, error = %source, "failed to validate job status");
            ApiError::internal(anyhow!(
                "failed to validate job status for '{job_id}': {source}"
            ))
        }
        UpsertPatchError::JobNotRunning { job_id, .. } => {
            error!(job_id = %job_id, "job not running when recording patch artifacts");
            ApiError::bad_request("job_id must reference a running job to record emitted artifacts")
        }
        UpsertPatchError::PatchNotFound { patch_id, .. } => {
            error!(patch_id = %patch_id, "patch not found");
            ApiError::not_found(format!("patch '{patch_id}' not found"))
        }
        UpsertPatchError::MergeRequestLookup { patch_id, source } => {
            error!(
                patch_id = %patch_id,
                error = %source,
                "failed to load merge-request issues for patch"
            );
            ApiError::internal(anyhow!(
                "failed to load merge-request issues for '{patch_id}': {source}"
            ))
        }
        UpsertPatchError::MergeRequestUpdate {
            patch_id,
            issue_id,
            source,
        } => {
            error!(
                patch_id = %patch_id,
                issue_id = %issue_id,
                error = %source,
                "failed to update merge-request issue for patch"
            );
            ApiError::internal(anyhow!(
                "failed to update merge-request issue '{issue_id}' for '{patch_id}': {source}"
            ))
        }
        UpsertPatchError::Store { source } => {
            error!(error = %source, "patch store operation failed");
            ApiError::internal(anyhow!("patch store error: {source}"))
        }
        UpsertPatchError::EmitArtifacts { job_id, source } => {
            error!(job_id = %job_id, error = %source, "failed to emit artifacts");
            ApiError::internal(anyhow!("failed to emit artifacts for '{job_id}': {source}"))
        }
    }
}

fn map_patch_error(err: StoreError, patch_id: Option<&PatchId>) -> ApiError {
    match err {
        StoreError::PatchNotFound(id) => {
            error!(patch_id = %id, "patch not found");
            ApiError::not_found(format!("patch '{id}' not found"))
        }
        other => {
            let patch_id = patch_id.map(|id| id.to_string()).unwrap_or_default();
            error!(
                patch_id = %patch_id,
                error = %other,
                "patch store operation failed"
            );
            ApiError::internal(anyhow!("patch store error: {other}"))
        }
    }
}
