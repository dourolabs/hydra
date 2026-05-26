use crate::domain::{
    actors::{Actor, ActorRef},
    patches::{GithubPr, PatchStatus},
};
use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::restrictions::MergeAuthorizationRestriction;
use crate::policy::{PolicyViolation, Restriction};
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
use hydra_common::{
    HydraId, PatchId,
    api::v1::{
        self, ApiError,
        merge_check::{MergeBlockedError, MergeCheckOk, MergeCheckResponse},
        pagination::{compute_next_cursor, effective_limit},
    },
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
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for PatchVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((patch_id, version)) =
            Path::<(PatchId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
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
    let expected_creator: hydra_common::api::v1::users::Username = actor.creator.clone().into();
    if payload.patch.creator != expected_creator {
        return Err(ApiError::bad_request(format!(
            "patch creator '{}' does not match authenticated actor's creator '{}'",
            payload.patch.creator.as_str(),
            expected_creator.as_str(),
        )));
    }
    let (patch_id, version) = state
        .upsert_patch_from_request(ActorRef::from(&actor), None, payload)
        .await
        .map_err(map_upsert_patch_error)?;

    info!(patch_id = %patch_id, "create_patch completed");
    Ok(Json(v1::patches::UpsertPatchResponse::new(
        patch_id, version,
    )))
}

pub async fn update_patch(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    PatchIdPath(patch_id): PatchIdPath,
    Json(payload): Json<v1::patches::UpsertPatchRequest>,
) -> Result<Json<v1::patches::UpsertPatchResponse>, ApiError> {
    info!(patch_id = %patch_id, "update_patch invoked");
    let (patch_id, version) = state
        .upsert_patch_from_request(ActorRef::from(&actor), Some(patch_id), payload)
        .await
        .map_err(map_upsert_patch_error)?;

    info!(patch_id = %patch_id, "update_patch completed");
    Ok(Json(v1::patches::UpsertPatchResponse::new(
        patch_id, version,
    )))
}

/// Preflight check: would calling `hydra patches merge` against this patch
/// succeed *right now* for the calling actor? Runs the same
/// `merge_authorization` restriction the write path runs on
/// `PatchStatus::Merged` transitions, but read-only — no state changes.
///
/// Returns `200` with `{ "ok": true }` if the merge would be allowed, or
/// `422 Unprocessable Entity` carrying a [`MergeBlockedError`] body
/// otherwise. The 422 status is documented in
/// `/designs/merge-time-constraints.md` §4.3 / §4.5 — it is NOT 400
/// (request well-formed) and NOT 403 (actor authorisation is only one of
/// two layers).
pub async fn merge_check(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<MergeCheckResponse, ApiError> {
    info!(patch_id = %patch_id, "merge_check invoked");

    // 1. Load current patch (404 if missing). Reuses the same error mapping
    //    as sibling read handlers.
    let current = state
        .get_patch(&patch_id, false)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    // 2. Build a context that mimics the write-path's `UpdatePatch`
    //    operation transitioning into `Merged`: old = current patch, new =
    //    same patch with status flipped to `Merged`. The restriction is the
    //    authoritative read-only evaluator — handing it the same shape the
    //    write path would supply guarantees parity (see §4.3).
    let old_patch = current.item.clone();
    let mut new_patch = current.item;
    new_patch.status = PatchStatus::Merged;

    let actor_ref = ActorRef::from(&actor);
    let payload = OperationPayload::Patch {
        patch_id: Some(patch_id.clone()),
        new: new_patch,
        old: Some(old_patch),
    };
    let ctx = RestrictionContext {
        operation: Operation::UpdatePatch,
        actor: &actor_ref,
        payload: &payload,
        store: state.store(),
    };

    // 3. Evaluate. The restriction is the single source of truth; we
    //    invoke it directly (not via `PolicyEngine::check_restrictions`)
    //    because the preflight surface answers ONLY the merge_authorization
    //    question — other restrictions don't belong in the response shape.
    let restriction = MergeAuthorizationRestriction::new();
    match restriction.evaluate(&ctx).await {
        Ok(()) => {
            info!(patch_id = %patch_id, "merge_check completed: allowed");
            Ok(MergeCheckResponse::Ok(MergeCheckOk::allowed()))
        }
        Err(violation) => {
            let body = parse_merge_blocked_violation(&violation, &patch_id)?;
            info!(
                patch_id = %patch_id,
                blocked_at_layer = ?body.blocked_at_layer,
                "merge_check completed: blocked"
            );
            Ok(MergeCheckResponse::Blocked(body))
        }
    }
}

/// Translate a `merge_authorization` `PolicyViolation` into the structured
/// [`MergeBlockedError`] wire body.
///
/// The restriction serialises its block response as JSON into
/// `PolicyViolation::message`; preflight just round-trips it back. Anything
/// else (an internal failure, e.g. "failed to load repository") arrives as a
/// non-JSON message — surface that as `500 Internal Server Error` because
/// preflight cannot distinguish it from a real block without the parse.
fn parse_merge_blocked_violation(
    violation: &PolicyViolation,
    patch_id: &PatchId,
) -> Result<MergeBlockedError, ApiError> {
    serde_json::from_str::<MergeBlockedError>(&violation.message).map_err(|err| {
        error!(
            patch_id = %patch_id,
            policy = %violation.policy_name,
            error = %err,
            message = %violation.message,
            "merge_authorization produced a non-MergeBlockedError violation"
        );
        ApiError::internal(format!(
            "merge_authorization internal failure: {}",
            violation.message
        ))
    })
}

pub async fn get_patch(
    State(state): State<AppState>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<v1::patches::PatchVersionRecord>, ApiError> {
    info!(patch_id = %patch_id, "get_patch invoked");
    let patch = state
        .get_patch(&patch_id, false)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let object_id = HydraId::from(patch_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to fetch labels for patch");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(patch_id = %patch_id, "get_patch completed");
    let response = v1::patches::PatchVersionRecord::new(
        patch_id,
        patch.version,
        patch.timestamp,
        patch.item.into(),
        patch.actor,
        patch.creation_time,
        labels,
    );
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

    let object_id = HydraId::from(patch_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to fetch labels for patch");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::patches::PatchVersionRecord::new(
                patch_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
                version.actor,
                version.creation_time,
                labels.clone(),
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
    PatchVersionPath {
        patch_id,
        version: raw_version,
    }: PatchVersionPath,
) -> Result<Json<v1::patches::PatchVersionRecord>, ApiError> {
    info!(patch_id = %patch_id, raw_version = raw_version.as_i64(), "get_patch_version invoked");
    let versions = state
        .get_patch_versions(&patch_id)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version = super::resolve_version(raw_version, max_version, "patch", patch_id.as_ref())?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!("patch '{patch_id}' version {version} not found"))
        })?;

    let object_id = HydraId::from(patch_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to fetch labels for patch");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let response = v1::patches::PatchVersionRecord::new(
        patch_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
        entry.actor,
        entry.creation_time,
        labels,
    );
    info!(patch_id = %patch_id, version, "get_patch_version completed");
    Ok(Json(response))
}

pub async fn list_patches(
    State(state): State<AppState>,
    Query(query): Query<v1::patches::SearchPatchesQuery>,
) -> Result<Json<v1::patches::ListPatchesResponse>, ApiError> {
    info!(query = ?query.q, include_deleted = ?query.include_deleted, "list_patches invoked");

    let patches = state
        .list_patches_with_query(&query)
        .await
        .map_err(|err| map_patch_error(err, None))?;

    // Batch-fetch labels for all patches in a single query
    let object_ids: Vec<HydraId> = patches
        .iter()
        .map(|(id, _)| HydraId::from(id.clone()))
        .collect();
    let labels_map = state
        .get_labels_for_objects(&object_ids)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to batch-fetch labels for patches");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    let eff_limit = effective_limit(query.limit);
    let mut records: Vec<v1::patches::PatchSummaryRecord> = patches
        .into_iter()
        .map(|(id, versioned)| {
            let object_id = HydraId::from(id.clone());
            let labels = labels_map.get(&object_id).cloned().unwrap_or_default();
            let full_record = v1::patches::PatchVersionRecord::new(
                id,
                versioned.version,
                versioned.timestamp,
                versioned.item.into(),
                versioned.actor,
                versioned.creation_time,
                labels,
            );
            v1::patches::PatchSummaryRecord::from(&full_record)
        })
        .collect();

    let next_cursor = compute_next_cursor(
        &mut records,
        eff_limit,
        |r| &r.timestamp,
        |r| r.patch_id.as_ref(),
    );

    let total_count = if query.count == Some(true) {
        let count = state
            .count_patches(&query)
            .await
            .map_err(|err| map_patch_error(err, None))?;
        Some(count)
    } else {
        None
    };

    let mut response = v1::patches::ListPatchesResponse::new(records);
    response.next_cursor = next_cursor;
    response.total_count = total_count;
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
        .get_patch(&patch_id, false)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;
    let github = patch
        .item
        .github
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("patch does not have a GitHub pull request"))?;

    let token = actor.get_github_token(&state).await?;
    let asset_name = resolve_asset_name(&query, &headers, &patch_id);
    let upload_url = build_upload_url(state.config.github_api_base_url(), github, &asset_name)?;
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
        .header(USER_AGENT, "hydra-server")
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

/// Derive the GitHub upload URL from the configured API base URL.
///
/// Note: This checks the *API base URL* host (e.g. `api.github.com`), not a repository
/// remote URL. This is API endpoint rewriting, not repository detection, so the
/// `Repository::is_github()` / `Repository::github_owner_repo()` helpers don't apply here.
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
            ApiError::bad_request("actor must reference a running job")
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
        UpsertPatchError::Store { source } => {
            error!(error = %source, "patch store operation failed");
            ApiError::internal(anyhow!("patch store error: {source}"))
        }
        UpsertPatchError::DuplicateBranchName {
            existing_patch_id,
            branch_name,
        } => ApiError::conflict(format!(
            "Can't create patch because an open patch '{existing_patch_id}' already exists \
             for branch '{branch_name}'. Consider updating that patch with: \
             hydra patches update {existing_patch_id}"
        )),
        UpsertPatchError::InvalidActorForReview { actor, reason } => {
            warn!(actor = ?actor, "rejected review-author request from non-principal actor");
            ApiError::bad_request(reason)
        }
        UpsertPatchError::PolicyViolation(violation) => ApiError::bad_request(violation.message),
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
    Extension(actor): Extension<Actor>,
    PatchIdPath(patch_id): PatchIdPath,
) -> Result<Json<v1::patches::PatchVersionRecord>, ApiError> {
    info!(patch_id = %patch_id, "delete_patch invoked");
    state
        .delete_patch(&patch_id, ActorRef::from(&actor))
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let patch = state
        .get_patch(&patch_id, true)
        .await
        .map_err(|err| map_patch_error(err, Some(&patch_id)))?;

    let object_id = HydraId::from(patch_id.clone());
    let labels = state
        .get_labels_for_object(&object_id)
        .await
        .map_err(|err| {
            error!(patch_id = %patch_id, error = %err, "failed to fetch labels for patch");
            ApiError::internal(anyhow!("failed to fetch labels: {err}"))
        })?;

    info!(patch_id = %patch_id, "delete_patch completed");
    let response = v1::patches::PatchVersionRecord::new(
        patch_id,
        patch.version,
        patch.timestamp,
        patch.item.into(),
        patch.actor,
        patch.creation_time,
        labels,
    );
    Ok(Json(response))
}

#[cfg(test)]
mod merge_check_tests {
    //! Handler-level tests for `POST /v1/patches/:patch_id/merge_check`.
    //!
    //! These call the handler function directly with constructed extractor
    //! wrappers (cheaper than spawning the full router/test-server). The
    //! handler's only interaction with HTTP is the `IntoResponse` impl on
    //! `MergeCheckResponse` and `ApiError`, both of which are covered by
    //! upstream unit tests; verifying the body + status mapping here would
    //! duplicate them.
    use super::*;
    use crate::domain::actors::Actor;
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::domain::users::Username;
    use crate::test_utils::{TestStateHandles, test_state_handles};
    use axum::http::StatusCode;
    use chrono::Utc;
    use hydra_common::ActorRef as CommonActorRef;
    use hydra_common::Principal as ApiPrincipal;
    use hydra_common::api::v1::merge_check::{
        BlockedAtLayer, MergeBlockedError, MergeBlockedReason, MergeCheckResponse,
    };
    use hydra_common::api::v1::repositories::{
        AssigneeRef, MergePolicy, MergerRule, ReviewerGroup,
    };
    use hydra_common::api::v1::users::Username as ApiUsername;
    use hydra_common::{RepoName, Repository};

    fn repo_name() -> RepoName {
        RepoName::new("octo", "repo").expect("valid repo name")
    }

    fn user_principal(name: &str) -> AssigneeRef {
        AssigneeRef::Static(ApiPrincipal::User {
            name: ApiUsername::try_new(name).unwrap_or_else(|_| ApiUsername::from(name)),
        })
    }

    fn actor_for(username: &str) -> Actor {
        Actor::new_for_user(Username::from(username)).0
    }

    fn approval(author: &str) -> Review {
        Review::new(
            "LGTM".to_string(),
            true,
            ApiPrincipal::User {
                name: ApiUsername::try_new(author).unwrap_or_else(|_| ApiUsername::from(author)),
            },
            Some(Utc::now()),
        )
    }

    fn open_patch(reviews: Vec<Review>) -> Patch {
        Patch::new(
            "title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("author"),
            reviews,
            repo_name(),
            None,
            None,
            None,
            None,
        )
    }

    async fn seed_repo(handles: &TestStateHandles, policy: Option<MergePolicy>) {
        let mut repo = Repository::new("https://example.com/repo.git".to_string(), None, None);
        repo.merge_policy = policy;
        handles
            .store
            .as_ref()
            .add_repository(repo_name(), repo, &CommonActorRef::test())
            .await
            .expect("seed repository");
    }

    async fn seed_patch(handles: &TestStateHandles, patch: Patch) -> PatchId {
        let (patch_id, _) = handles
            .store
            .as_ref()
            .add_patch(patch, &CommonActorRef::test())
            .await
            .expect("seed patch");
        patch_id
    }

    async fn call_merge_check(
        state: &AppState,
        actor: Actor,
        patch_id: PatchId,
    ) -> Result<MergeCheckResponse, ApiError> {
        merge_check(
            State(state.clone()),
            Extension(actor),
            PatchIdPath(patch_id),
        )
        .await
    }

    #[tokio::test]
    async fn merge_check_returns_ok_when_no_merge_policy() {
        let handles = test_state_handles();
        seed_repo(&handles, None).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;

        let response = call_merge_check(&handles.state, actor_for("anyone"), patch_id)
            .await
            .expect("handler must succeed when no policy is configured");
        assert!(
            matches!(response, MergeCheckResponse::Ok(_)),
            "expected MergeCheckResponse::Ok, got {response:?}"
        );
    }

    #[tokio::test]
    async fn merge_check_blocks_when_reviewer_missing() {
        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("code-review".to_string()),
                any_of: vec![user_principal("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;

        let response = call_merge_check(&handles.state, actor_for("anyone"), patch_id.clone())
            .await
            .expect("handler returns Ok-wrapped MergeCheckResponse on block");

        let body = match response {
            MergeCheckResponse::Blocked(body) => body,
            other => panic!("expected Blocked, got {other:?}"),
        };
        assert_eq!(body.patch_id, patch_id);
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Reviews);
        assert_eq!(body.reasons.len(), 1);
        assert!(matches!(
            body.reasons[0],
            MergeBlockedReason::MissingApprovals { .. }
        ));
    }

    #[tokio::test]
    async fn merge_check_allows_after_non_stale_approval() {
        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user_principal("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![approval("reviewer")])).await;

        let response = call_merge_check(&handles.state, actor_for("anyone"), patch_id)
            .await
            .expect("handler must succeed once the approval lands");
        assert!(
            matches!(response, MergeCheckResponse::Ok(_)),
            "expected Ok, got {response:?}"
        );
    }

    #[tokio::test]
    async fn merge_check_blocks_when_actor_not_in_mergers() {
        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![user_principal("alice")],
            }),
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;

        let response = call_merge_check(&handles.state, actor_for("swe-x"), patch_id)
            .await
            .expect("handler returns Ok-wrapped MergeCheckResponse on block");

        let body = match response {
            MergeCheckResponse::Blocked(body) => body,
            other => panic!("expected Blocked, got {other:?}"),
        };
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Mergers);
        match &body.reasons[..] {
            [MergeBlockedReason::NotInMergers { actor, .. }] => {
                assert_eq!(actor, "swe-x");
            }
            other => panic!("expected single NotInMergers reason, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn merge_check_allows_listed_merger() {
        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![],
            mergers: Some(MergerRule {
                any_of: vec![user_principal("alice")],
            }),
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;

        let response = call_merge_check(&handles.state, actor_for("alice"), patch_id)
            .await
            .expect("alice is in mergers, must succeed");
        assert!(matches!(response, MergeCheckResponse::Ok(_)));
    }

    #[tokio::test]
    async fn merge_check_priority_gates_to_reviews_only() {
        // Both layers would fail simultaneously, but the response must
        // expose ONLY the reviews-layer reason — per design §4.5 the SWE
        // sees one priority layer at a time.
        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: None,
                any_of: vec![user_principal("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: Some(MergerRule {
                any_of: vec![user_principal("alice")],
            }),
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;

        let response = call_merge_check(&handles.state, actor_for("bob"), patch_id)
            .await
            .expect("handler returns Ok-wrapped MergeCheckResponse on block");

        let body = match response {
            MergeCheckResponse::Blocked(body) => body,
            other => panic!("expected Blocked, got {other:?}"),
        };
        assert_eq!(body.blocked_at_layer, BlockedAtLayer::Reviews);
        assert!(
            body.reasons
                .iter()
                .all(|r| matches!(r, MergeBlockedReason::MissingApprovals { .. })),
            "reviews-layer block must NOT carry NotInMergers reasons; \
             got: {:?}",
            body.reasons
        );
    }

    #[tokio::test]
    async fn merge_check_unknown_patch_id_is_404() {
        let handles = test_state_handles();
        seed_repo(&handles, None).await;
        let bogus = PatchId::new();

        let err = call_merge_check(&handles.state, actor_for("anyone"), bogus)
            .await
            .expect_err("unknown patch id must be a 404");
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    /// Parity test: the JSON body the preflight endpoint emits on a block
    /// must equal the JSON the restriction stuffs into
    /// `PolicyViolation::message` for the same inputs — that's the
    /// guarantee the design hangs on, since the CLI parses the same wire
    /// shape from both surfaces.
    #[tokio::test]
    async fn merge_check_blocked_body_matches_restriction_violation() {
        use crate::domain::actors::ActorRef as DomainActorRef;
        use crate::policy::Restriction;
        use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
        use crate::policy::restrictions::MergeAuthorizationRestriction;

        let handles = test_state_handles();
        let policy = MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("code-review".to_string()),
                any_of: vec![user_principal("reviewer")],
                count: 1,
                exclude_author: true,
            }],
            mergers: None,
        };
        seed_repo(&handles, Some(policy)).await;
        let patch_id = seed_patch(&handles, open_patch(vec![])).await;
        let actor = actor_for("author");

        let response = call_merge_check(&handles.state, actor.clone(), patch_id.clone())
            .await
            .expect("merge_check call must not error");
        let preflight_body = match response {
            MergeCheckResponse::Blocked(body) => body,
            other => panic!("expected Blocked, got {other:?}"),
        };

        // Reproduce the write path's restriction call exactly: same actor,
        // same store, same simulated UpdatePatch payload.
        let current = handles
            .state
            .get_patch(&patch_id, false)
            .await
            .expect("patch still present");
        let old_patch = current.item.clone();
        let mut new_patch = current.item;
        new_patch.status = PatchStatus::Merged;

        let actor_ref = DomainActorRef::from(&actor);
        let payload = OperationPayload::Patch {
            patch_id: Some(patch_id.clone()),
            new: new_patch,
            old: Some(old_patch),
        };
        let ctx = RestrictionContext {
            operation: Operation::UpdatePatch,
            actor: &actor_ref,
            payload: &payload,
            store: handles.state.store(),
        };
        let violation = MergeAuthorizationRestriction::new()
            .evaluate(&ctx)
            .await
            .expect_err("restriction must block on the same inputs");
        let restriction_body: MergeBlockedError = serde_json::from_str(&violation.message)
            .expect("restriction emits MergeBlockedError JSON");

        // Compare the two as JSON to keep the assertion focused on the
        // wire shape (not on Rust struct equality), since the wire shape is
        // the contract the CLI consumes from both surfaces.
        assert_eq!(
            serde_json::to_value(&preflight_body).unwrap(),
            serde_json::to_value(&restriction_body).unwrap(),
        );
    }
}
