use crate::{AppState, routes::jobs::ApiError, store::StoreError};
use anyhow::anyhow;
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::artifacts::{
    Artifact, ArtifactKind, ArtifactRecord, ListArtifactsResponse, SearchArtifactsQuery,
    UpsertArtifactRequest, UpsertArtifactResponse,
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
        query = ?query.q,
        "list_artifacts invoked"
    );

    let search_term = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());

    let store_read = state.store.read().await;
    let artifacts = store_read
        .list_artifacts()
        .await
        .map_err(|err| map_store_error(err, None))?;

    let filtered = artifacts
        .into_iter()
        .filter(|(id, artifact)| {
            artifact_matches(&query.artifact_type, search_term.as_deref(), id, artifact)
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
    let mut store = state.store.write().await;
    let artifact_id = match artifact_id {
        Some(id) => match store.update_artifact(&id, payload.artifact).await {
            Ok(()) => id,
            Err(err) => return Err(map_store_error(err, Some(&id))),
        },
        None => store
            .add_artifact(payload.artifact)
            .await
            .map_err(|err| map_store_error(err, None))?,
    };

    info!(artifact_id = %artifact_id, "artifact stored successfully");

    Ok(Json(UpsertArtifactResponse { artifact_id }))
}

fn artifact_matches(
    kind_filter: &Option<ArtifactKind>,
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

    if let Some(term) = search_term {
        let lower_id = artifact_id.to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return match artifact {
            Artifact::Patch { diff } => diff.to_lowercase().contains(term),
            Artifact::Issue { description } => description.to_lowercase().contains(term),
        };
    }

    true
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
