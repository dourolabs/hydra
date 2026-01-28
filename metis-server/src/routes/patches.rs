use crate::domain::patches::{
    ListPatchesResponse, Patch, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
};
use crate::{
    app::{AppState, UpsertPatchError},
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
    api::v1::{self, ApiError},
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
    Json(payload): Json<v1::patches::UpsertPatchRequest>,
) -> Result<Json<v1::patches::UpsertPatchResponse>, ApiError> {
    info!("create_patch invoked");
    let request: UpsertPatchRequest = payload.into();
    let patch_id = state
        .upsert_patch(None, request)
        .await
        .map_err(map_upsert_patch_error)?;

    info!(patch_id = %patch_id, "create_patch completed");
    Ok(Json(v1::patches::UpsertPatchResponse::new(patch_id)))
}

pub async fn update_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
    Json(payload): Json<v1::patches::UpsertPatchRequest>,
) -> Result<Json<v1::patches::UpsertPatchResponse>, ApiError> {
    info!(patch_id = %patch_id, "update_patch invoked");
    let request: UpsertPatchRequest = payload.into();
    let patch_id = state
        .upsert_patch(Some(patch_id), request)
        .await
        .map_err(map_upsert_patch_error)?;

    info!(patch_id = %patch_id, "update_patch completed");
    Ok(Json(v1::patches::UpsertPatchResponse::new(patch_id)))
}

pub async fn get_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<v1::patches::PatchRecord>, ApiError> {
    info!(patch_id = %patch_id, "get_patch invoked");
    let patch = state
        .get_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    info!(patch_id = %patch_id, "get_patch completed");
    let response: v1::patches::PatchRecord = PatchRecord::new(patch_id, patch.item).into();
    Ok(Json(response))
}

pub async fn list_patches(
    State(state): State<AppState>,
    Query(query): Query<v1::patches::SearchPatchesQuery>,
) -> Result<Json<v1::patches::ListPatchesResponse>, ApiError> {
    info!(query = ?query.q, "list_patches invoked");
    let query: SearchPatchesQuery = query.into();

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());

    let patches = state
        .list_patches()
        .await
        .map_err(|err| map_patch_error(err, None))?;

    let filtered = patches
        .into_iter()
        .filter(|(id, patch)| patch_matches(search_term.as_deref(), id, &patch.item))
        .map(|(id, patch)| PatchRecord::new(id, patch.item))
        .collect();

    let response: v1::patches::ListPatchesResponse = ListPatchesResponse::new(filtered).into();
    info!(
        query = ?query.q,
        returned = response.patches.len(),
        "list_patches completed"
    );
    Ok(Json(response))
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
                .service_repo_name
                .to_string()
                .to_lowercase()
                .contains(term)
            || patch.diff.to_lowercase().contains(term)
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
            error!(job_id = %job_id, "job not running when recording patch metadata");
            ApiError::bad_request("created_by must reference a running job")
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
        UpsertPatchError::GithubAppUnavailable => {
            error!("github app not configured for patch sync");
            ApiError::internal(anyhow!("github app not configured"))
        }
        UpsertPatchError::GithubInstallationLookup {
            owner,
            repo,
            source,
        } => {
            error!(
                owner = %owner,
                repo = %repo,
                error = %source,
                "failed to lookup github installation"
            );
            ApiError::internal(anyhow!(
                "failed to lookup github installation for '{owner}/{repo}': {source}"
            ))
        }
        UpsertPatchError::GithubInstallationClient {
            owner,
            repo,
            source,
        } => {
            error!(
                owner = %owner,
                repo = %repo,
                error = %source,
                "failed to create github installation client"
            );
            ApiError::internal(anyhow!(
                "failed to create github installation client for '{owner}/{repo}': {source}"
            ))
        }
        UpsertPatchError::GithubHeadRefMissing => {
            error!("missing github head ref for patch sync");
            ApiError::bad_request("github head ref must be provided")
        }
        UpsertPatchError::GithubBaseRefMissing => {
            error!("missing github base ref for patch sync");
            ApiError::bad_request("github base ref must be provided")
        }
        UpsertPatchError::GithubRepositoryLookup { repo_name, source } => match source {
            StoreError::RepositoryNotFound(_) => {
                error!(repo_name = %repo_name, "repository not found for github sync");
                ApiError::bad_request(format!("repository '{repo_name}' not found"))
            }
            other => {
                error!(
                    repo_name = %repo_name,
                    error = %other,
                    "failed to load repository for github sync"
                );
                ApiError::internal(anyhow!(
                    "failed to load repository '{repo_name}' for github sync: {other}"
                ))
            }
        },
        UpsertPatchError::GithubPullRequestUpdate {
            owner,
            repo,
            number,
            source,
        } => {
            error!(
                owner = %owner,
                repo = %repo,
                number = %number,
                error = %source,
                "failed to update github pull request"
            );
            ApiError::internal(anyhow!(
                "failed to update github pull request '{owner}/{repo}#{number}': {source}"
            ))
        }
        UpsertPatchError::GithubPullRequestCreate {
            owner,
            repo,
            source,
        } => {
            error!(
                owner = %owner,
                repo = %repo,
                error = %source,
                "failed to create github pull request"
            );
            ApiError::internal(anyhow!(
                "failed to create github pull request for '{owner}/{repo}': {source}"
            ))
        }
        UpsertPatchError::Store { source } => {
            error!(error = %source, "patch store operation failed");
            ApiError::internal(anyhow!("patch store error: {source}"))
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
