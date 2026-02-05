use crate::domain::{
    actors::Actor,
    patches::{
        GithubPr, ListPatchesResponse, Patch, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
    },
};
use crate::{
    app::{AppState, UpsertPatchError},
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Extension, Json, async_trait,
    body::Bytes,
    extract::{FromRequestParts, Path, Query, State},
    http::{HeaderMap, header::CONTENT_DISPOSITION, request::Parts},
};
use metis_common::{
    PatchId, VersionNumber,
    api::v1::{self, ApiError},
};
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue, USER_AGENT,
};
use serde::Deserialize;
use std::path::Path as FilePath;
use tracing::{error, info, warn};

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

#[derive(Debug, Clone)]
pub struct PatchVersionPath {
    pub patch_id: PatchId,
    pub version: VersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for PatchVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((patch_id, version)) =
            Path::<(PatchId, VersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self { patch_id, version })
    }
}

pub async fn create_patch(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<v1::patches::UpsertPatchRequest>,
) -> Result<Json<v1::patches::UpsertPatchResponse>, ApiError> {
    info!("create_patch invoked");
    let request: UpsertPatchRequest = payload.into();
    let patch_id = state
        .upsert_patch(Some(&actor), None, request)
        .await
        .map_err(map_upsert_patch_error)?;

    info!(patch_id = %patch_id, "create_patch completed");
    Ok(Json(v1::patches::UpsertPatchResponse::new(patch_id)))
}

pub async fn update_patch(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    PatchIdPath(patch_id): PatchIdPath,
    Json(payload): Json<v1::patches::UpsertPatchRequest>,
) -> Result<Json<v1::patches::UpsertPatchResponse>, ApiError> {
    info!(patch_id = %patch_id, "update_patch invoked");
    let request: UpsertPatchRequest = payload.into();
    let patch_id = state
        .upsert_patch(Some(&actor), Some(patch_id), request)
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

pub async fn list_patch_versions(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<v1::patches::ListPatchVersionsResponse>, ApiError> {
    info!(patch_id = %patch_id, "list_patch_versions invoked");
    let versions = state
        .get_patch_versions(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::patches::PatchVersionRecord::new(
                patch_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
            )
        })
        .collect();

    let response = v1::patches::ListPatchVersionsResponse::new(records);
    info!(
        patch_id = %patch_id,
        returned = response.versions.len(),
        "list_patch_versions completed"
    );
    Ok(Json(response))
}

pub async fn get_patch_version(
    State(state): State<AppState>,
    PatchVersionPath { patch_id, version }: PatchVersionPath,
) -> Result<Json<v1::patches::PatchVersionRecord>, ApiError> {
    info!(patch_id = %patch_id, version, "get_patch_version invoked");
    let versions = state
        .get_patch_versions(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!("patch '{patch_id}' version {version} not found"))
        })?;

    let response = v1::patches::PatchVersionRecord::new(
        patch_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
    );
    info!(patch_id = %patch_id, version, "get_patch_version completed");
    Ok(Json(response))
}

pub async fn list_patches(
    State(state): State<AppState>,
    Query(query): Query<v1::patches::SearchPatchesQuery>,
) -> Result<Json<v1::patches::ListPatchesResponse>, ApiError> {
    info!(query = ?query.q, include_deleted = ?query.include_deleted, "list_patches invoked");
    let query: SearchPatchesQuery = query.into();

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let include_deleted = query.include_deleted.unwrap_or(false);

    let patches = state
        .list_patches_with_deleted(include_deleted)
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

pub async fn create_patch_asset(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    PatchIdPath(patch_id): PatchIdPath,
    Query(query): Query<v1::patches::CreatePatchAssetQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<v1::patches::CreatePatchAssetResponse>, ApiError> {
    info!(patch_id = %patch_id, "create_patch_asset invoked");

    if body.is_empty() {
        return Err(ApiError::bad_request("asset payload must not be empty"));
    }

    let patch = state
        .get_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;
    let github = patch
        .item
        .github
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("patch does not have a GitHub pull request"))?;

    let token = actor.get_github_token(&state).await?;
    let asset_name = resolve_asset_name(&query, &headers, &patch_id);
    let upload_url = build_upload_url(state.config.github_app.api_base_url(), github, &asset_name)?;
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("application/octet-stream");
    let content_type = if content_type.parse::<mime::Mime>().is_ok() {
        content_type
    } else {
        warn!(
            patch_id = %patch_id,
            content_type = %content_type,
            "invalid content type for github asset upload; using default"
        );
        "application/octet-stream"
    };
    let body_len =
        u64::try_from(body.len()).map_err(|_| ApiError::internal("asset payload too large"))?;
    let content_length = HeaderValue::from_str(&body_len.to_string()).map_err(|err| {
        ApiError::internal(format!(
            "invalid content length for github asset upload: {err}"
        ))
    })?;
    let body = reqwest::Body::from(body);

    let response = reqwest::Client::new()
        .post(upload_url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, "metis-server")
        .header(AUTHORIZATION, format!("Bearer {}", token.github_token))
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content_length)
        .body(body)
        .send()
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to upload patch asset");
            ApiError::internal(format!("failed to upload patch asset: {err}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        error!(
            patch_id = %patch_id,
            status = %status,
            body = %error_body,
            "github asset upload failed"
        );
        let message = if error_body.trim().is_empty() {
            format!("github asset upload failed with status {status}")
        } else {
            format!("github asset upload failed with status {status}: {error_body}")
        };
        return Err(ApiError::internal(message));
    }

    let payload = response
        .json::<GithubAssetUploadResponse>()
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to decode github response");
            ApiError::internal(format!("failed to decode github response: {err}"))
        })?;

    let asset_url = payload.asset_url().ok_or_else(|| {
        ApiError::internal("github asset upload response did not include an asset url")
    })?;

    info!(patch_id = %patch_id, asset_url = %asset_url, "create_patch_asset completed");
    Ok(Json(v1::patches::CreatePatchAssetResponse::new(asset_url)))
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

#[derive(Debug, Deserialize)]
struct GithubAssetUploadResponse {
    url: Option<String>,
    browser_download_url: Option<String>,
    html_url: Option<String>,
    markdown: Option<String>,
}

impl GithubAssetUploadResponse {
    fn asset_url(self) -> Option<String> {
        if let Some(url) = self.url {
            return Some(url);
        }
        if let Some(url) = self.browser_download_url {
            return Some(url);
        }
        if let Some(url) = self.html_url {
            return Some(url);
        }
        self.markdown
            .as_deref()
            .and_then(extract_url_from_markdown)
            .map(ToString::to_string)
    }
}

fn extract_url_from_markdown(markdown: &str) -> Option<&str> {
    let start = markdown.find('(')?;
    let end = markdown[start + 1..].find(')')?;
    Some(&markdown[start + 1..start + 1 + end])
}

fn resolve_asset_name(
    query: &v1::patches::CreatePatchAssetQuery,
    headers: &HeaderMap,
    patch_id: &PatchId,
) -> String {
    query
        .name
        .as_ref()
        .and_then(|value| sanitize_filename(value))
        .or_else(|| filename_from_content_disposition(headers))
        .unwrap_or_else(|| format!("patch-{patch_id}-asset.bin"))
}

fn filename_from_content_disposition(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(CONTENT_DISPOSITION)?.to_str().ok()?;
    for part in value.split(';') {
        let part = part.trim();
        if let Some(filename) = part.strip_prefix("filename=") {
            let filename = filename.trim_matches('"');
            if !filename.is_empty() {
                return sanitize_filename(filename);
            }
        }
    }
    None
}

fn sanitize_filename(value: &str) -> Option<String> {
    let filename = FilePath::new(value).file_name()?.to_string_lossy();
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_upload_url(
    api_base_url: &str,
    github: &GithubPr,
    name: &str,
) -> Result<reqwest::Url, ApiError> {
    let mut base = github_upload_base_url(api_base_url)?;
    base.set_path(&format!(
        "/repos/{}/{}/issues/{}/comments/attachments",
        github.owner, github.repo, github.number
    ));
    base.set_query(None);
    base.query_pairs_mut().append_pair("name", name);
    Ok(base)
}

fn github_upload_base_url(api_base_url: &str) -> Result<reqwest::Url, ApiError> {
    let mut url = reqwest::Url::parse(api_base_url).map_err(|err| {
        ApiError::internal(format!(
            "invalid github api base url '{api_base_url}': {err}"
        ))
    })?;

    if url.host_str() == Some("api.github.com") {
        url.set_host(Some("uploads.github.com"))
            .map_err(|err| ApiError::internal(format!("invalid github upload url: {err}")))?;
        url.set_path("/");
        url.set_query(None);
        return Ok(url);
    }

    if url.path().ends_with("/api/v3") {
        let trimmed = url.path().trim_end_matches("/api/v3");
        let mut new_path = String::new();
        if !trimmed.is_empty() {
            new_path.push_str(trimmed.trim_end_matches('/'));
        }
        new_path.push_str("/api/uploads");
        url.set_path(&new_path);
        url.set_query(None);
        return Ok(url);
    }

    url.set_query(None);
    Ok(url)
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
        UpsertPatchError::MergeRequestCreate { patch_id, source } => {
            error!(
                patch_id = %patch_id,
                error = %source,
                "failed to create merge-request issue for patch"
            );
            ApiError::internal(anyhow!(
                "failed to create merge-request issue for '{patch_id}': {source}"
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
        UpsertPatchError::GithubActorMissing => {
            error!("github sync requested without authenticated actor");
            ApiError::internal(anyhow!("github sync requires an authenticated actor"))
        }
        UpsertPatchError::GithubTokenLookup { actor, message } => {
            error!(
                actor = %actor,
                error = %message,
                "failed to fetch github token for patch sync"
            );
            ApiError::unauthorized("github token unavailable")
        }
        UpsertPatchError::GithubUserClient { actor, source } => {
            error!(
                actor = %actor,
                error = %source,
                "failed to create github client for patch sync"
            );
            ApiError::internal(anyhow!(
                "failed to create github client for '{actor}': {source}"
            ))
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

pub async fn delete_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<v1::patches::PatchRecord>, ApiError> {
    info!(patch_id = %patch_id, "delete_patch invoked");
    state
        .delete_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let patch = state
        .get_patch(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    info!(patch_id = %patch_id, "delete_patch completed");
    let response: v1::patches::PatchRecord = PatchRecord::new(patch_id, patch.item).into();
    Ok(Json(response))
}
