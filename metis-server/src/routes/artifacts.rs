use crate::{
    AppState,
    routes::jobs::ApiError,
    store::{Status, StoreError},
};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use chrono::Utc;
use metis_common::artifacts::{
    Artifact, ArtifactKind, ArtifactRecord, IssueStatus, IssueType, ListArtifactsResponse,
    SearchArtifactsQuery, UpsertArtifactRequest, UpsertArtifactResponse,
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct ArtifactIdPath(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for ArtifactIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(artifact_id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = artifact_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("artifact_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}

pub async fn create_artifact(
    State(state): State<AppState>,
    Json(payload): Json<UpsertArtifactRequest>,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    info!("create_artifact invoked");
    upsert_artifact_internal(state, None, payload).await
}

pub async fn update_artifact(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
    Json(payload): Json<UpsertArtifactRequest>,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    info!(artifact_id = %artifact_id, "update_artifact invoked");
    upsert_artifact_internal(state, Some(artifact_id), payload).await
}

pub async fn get_artifact(
    State(state): State<AppState>,
    ArtifactIdPath(artifact_id): ArtifactIdPath,
) -> Result<Json<ArtifactRecord>, ApiError> {
    info!(artifact_id = %artifact_id, "get_artifact invoked");
    let store_read = state.store.read().await;
    let artifact = store_read
        .get_artifact(&artifact_id)
        .await
        .map_err(|err| map_store_error(err, Some(&artifact_id)))?;

    Ok(Json(ArtifactRecord {
        id: artifact_id,
        artifact,
    }))
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    Query(query): Query<SearchArtifactsQuery>,
) -> Result<Json<ListArtifactsResponse>, ApiError> {
    info!(
        artifact_type = ?query.artifact_type,
        issue_type = ?query.issue_type,
        status = ?query.status,
        query = ?query.q,
        "list_artifacts invoked"
    );

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let artifacts = if let Some(kind) = query.artifact_type {
        store_read
            .list_artifacts_with_type(kind)
            .await
            .map_err(|err| map_store_error(err, None))?
    } else {
        store_read
            .list_artifacts()
            .await
            .map_err(|err| map_store_error(err, None))?
    };

    let filtered = artifacts
        .into_iter()
        .filter(|(id, artifact)| {
            artifact_matches(
                &query.artifact_type,
                query.issue_type,
                query.status,
                search_term.as_deref(),
                id,
                artifact,
            )
        })
        .map(|(id, artifact)| ArtifactRecord { id, artifact })
        .collect();

    Ok(Json(ListArtifactsResponse {
        artifacts: filtered,
    }))
}

async fn upsert_artifact_internal(
    state: AppState,
    artifact_id: Option<String>,
    payload: UpsertArtifactRequest,
) -> Result<Json<UpsertArtifactResponse>, ApiError> {
    let UpsertArtifactRequest { artifact, job_id } = payload;

    let mut store = state.store.write().await;
    let artifact_id = match artifact_id {
        Some(id) => {
            if job_id.is_some() {
                return Err(ApiError::bad_request(
                    "job_id may only be provided when creating an artifact",
                ));
            }
            match store.update_artifact(&id, artifact).await {
                Ok(()) => id,
                Err(err) => return Err(map_store_error(err, Some(&id))),
            }
        }
        None => {
            let job_id = job_id
                .as_ref()
                .map(|value| value.trim())
                .map(|value| value.to_string());

            if let Some(ref job_id) = job_id {
                if job_id.is_empty() {
                    return Err(ApiError::bad_request("job_id must not be empty"));
                }

                let status = store.get_status(job_id).await.map_err(|err| match err {
                    StoreError::TaskNotFound(id) => {
                        error!(job_id = %id, "job not found when creating artifact");
                        ApiError::not_found(format!("job '{id}' not found"))
                    }
                    other => {
                        error!(job_id = %job_id, error = %other, "failed to validate job status");
                        ApiError::internal(anyhow!(
                            "failed to validate job status for '{job_id}': {other}"
                        ))
                    }
                })?;

                if status != Status::Running {
                    return Err(ApiError::bad_request(
                        "job_id must reference a running job to record emitted artifacts",
                    ));
                }
            }

            let id = store
                .add_artifact(artifact)
                .await
                .map_err(|err| map_store_error(err, None))?;

            if let Some(job_id) = job_id {
                store
                    .emit_task_artifacts(&job_id, vec![id.clone()], Utc::now())
                    .await
                    .map_err(|err| map_emit_error(err, &job_id))?;
            }

            id
        }
    };

    info!(artifact_id = %artifact_id, "artifact stored successfully");

    Ok(Json(UpsertArtifactResponse { artifact_id }))
}

fn artifact_matches(
    kind_filter: &Option<ArtifactKind>,
    issue_type_filter: Option<IssueType>,
    status_filter: Option<IssueStatus>,
    search_term: Option<&str>,
    artifact_id: &str,
    artifact: &Artifact,
) -> bool {
    if let Some(kind) = kind_filter {
        let artifact_kind = ArtifactKind::from(artifact);
        if &artifact_kind != kind {
            return false;
        }
    }

    if let Some(issue_type) = issue_type_filter {
        match artifact {
            Artifact::Issue {
                issue_type: current,
                ..
            } if current == &issue_type => {}
            Artifact::Issue { .. } => return false,
            _ => return false,
        }
    }

    if let Some(status) = status_filter {
        match artifact {
            Artifact::Issue {
                status: current, ..
            } if current == &status => {}
            Artifact::Issue { .. } => return false,
            _ => return false,
        }
    }

    if let Some(term) = search_term {
        let lower_id = artifact_id.to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return match artifact {
            Artifact::Patch { diff, description } => {
                diff.to_lowercase().contains(term) || description.to_lowercase().contains(term)
            }
            Artifact::Issue {
                description,
                issue_type,
                status,
                ..
            } => {
                description.to_lowercase().contains(term)
                    || issue_type_matches(term, issue_type)
                    || issue_status_matches(term, status)
            }
            Artifact::Session {
                program,
                params,
                context,
                image,
                env_vars,
                dependencies,
                ..
            } => {
                program.to_lowercase().contains(term)
                    || params
                        .iter()
                        .any(|param| param.to_lowercase().contains(term))
                    || image.to_lowercase().contains(term)
                    || env_vars.iter().any(|(key, value)| {
                        key.to_lowercase().contains(term) || value.to_lowercase().contains(term)
                    })
                    || dependencies
                        .iter()
                        .any(|dependency| dependency.issue_id.to_lowercase().contains(term))
                    || match context {
                        metis_common::jobs::Bundle::GitRepository { url, .. } => {
                            url.to_lowercase().contains(term)
                        }
                        metis_common::jobs::Bundle::GitBundle { .. } => false,
                        metis_common::jobs::Bundle::TarGz { .. } => false,
                        metis_common::jobs::Bundle::None => false,
                    }
            }
        };
    }

    true
}

fn issue_type_matches(search_term: &str, issue_type: &IssueType) -> bool {
    issue_type.as_str() == search_term
}

fn issue_status_matches(search_term: &str, status: &IssueStatus) -> bool {
    status.as_str() == search_term
}

fn map_store_error(err: StoreError, artifact_id: Option<&str>) -> ApiError {
    match err {
        StoreError::ArtifactNotFound(id) => {
            error!(artifact_id = %id, "artifact not found");
            ApiError::not_found(format!("artifact '{id}' not found"))
        }
        other => {
            error!(
                artifact_id = artifact_id.unwrap_or_default(),
                error = %other,
                "artifact store operation failed"
            );
            ApiError::internal(anyhow!("artifact store error: {other}"))
        }
    }
}

fn map_emit_error(err: StoreError, job_id: &str) -> ApiError {
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
