use crate::domain::actors::Actor;
use crate::domain::documents::{SearchDocumentsQuery, UpsertDocumentRequest};
use crate::{
    app::{AppState, UpsertDocumentError},
    store::StoreError,
};
use anyhow::anyhow;
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use metis_common::{
    DocumentId, VersionNumber,
    api::v1::{self, ApiError},
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct DocumentIdPath(pub DocumentId);

#[async_trait]
impl<S> FromRequestParts<S> for DocumentIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(document_id) = Path::<DocumentId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(document_id))
    }
}

#[derive(Debug, Clone)]
pub struct DocumentVersionPath {
    pub document_id: DocumentId,
    pub version: VersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for DocumentVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((document_id, version)) =
            Path::<(DocumentId, VersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self {
            document_id,
            version,
        })
    }
}

pub async fn create_document(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<v1::documents::UpsertDocumentRequest>,
) -> Result<Json<v1::documents::UpsertDocumentResponse>, ApiError> {
    info!(actor = %actor.name(), "create_document invoked");
    let request: UpsertDocumentRequest = payload.into();
    let document_id = state
        .upsert_document(None, request.document)
        .await
        .map_err(map_upsert_document_error)?;

    info!(actor = %actor.name(), document_id = %document_id, "create_document completed");
    Ok(Json(v1::documents::UpsertDocumentResponse::new(
        document_id,
    )))
}

pub async fn update_document(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    DocumentIdPath(document_id): DocumentIdPath,
    Json(payload): Json<v1::documents::UpsertDocumentRequest>,
) -> Result<Json<v1::documents::UpsertDocumentResponse>, ApiError> {
    info!(actor = %actor.name(), document_id = %document_id, "update_document invoked");
    let request: UpsertDocumentRequest = payload.into();
    let document_id = state
        .upsert_document(Some(document_id.clone()), request.document)
        .await
        .map_err(map_upsert_document_error)?;

    info!(actor = %actor.name(), document_id = %document_id, "update_document completed");
    Ok(Json(v1::documents::UpsertDocumentResponse::new(
        document_id,
    )))
}

pub async fn get_document(
    State(state): State<AppState>,
    DocumentIdPath(document_id): DocumentIdPath,
) -> Result<Json<v1::documents::DocumentRecord>, ApiError> {
    info!(document_id = %document_id, "get_document invoked");
    let document = state
        .get_document(&document_id)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let response = v1::documents::DocumentRecord::new(document_id.clone(), document.item.into());
    info!(document_id = %document_id, "get_document completed");
    Ok(Json(response))
}

pub async fn list_documents(
    State(state): State<AppState>,
    Query(query): Query<v1::documents::SearchDocumentsQuery>,
) -> Result<Json<v1::documents::ListDocumentsResponse>, ApiError> {
    info!(query = ?query.q, path_prefix = ?query.path_prefix, created_by = ?query.created_by, "list_documents invoked");
    let query: SearchDocumentsQuery = query.into();
    let documents = state
        .list_documents(&query)
        .await
        .map_err(|err| map_document_error(err, None))?;

    let records = documents
        .into_iter()
        .map(|(id, versioned)| v1::documents::DocumentRecord::new(id, versioned.item.into()))
        .collect();

    let response = v1::documents::ListDocumentsResponse::new(records);
    info!(
        returned = response.documents.len(),
        "list_documents completed"
    );
    Ok(Json(response))
}

pub async fn list_document_versions(
    State(state): State<AppState>,
    DocumentIdPath(document_id): DocumentIdPath,
) -> Result<Json<v1::documents::ListDocumentVersionsResponse>, ApiError> {
    info!(document_id = %document_id, "list_document_versions invoked");
    let versions = state
        .get_document_versions(&document_id)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let records = versions
        .into_iter()
        .map(|version| {
            v1::documents::DocumentVersionRecord::new(
                document_id.clone(),
                version.version,
                version.timestamp,
                version.item.into(),
            )
        })
        .collect();

    let response = v1::documents::ListDocumentVersionsResponse::new(records);
    info!(document_id = %document_id, versions = response.versions.len(), "list_document_versions completed");
    Ok(Json(response))
}

pub async fn get_document_version(
    State(state): State<AppState>,
    DocumentVersionPath {
        document_id,
        version,
    }: DocumentVersionPath,
) -> Result<Json<v1::documents::DocumentVersionRecord>, ApiError> {
    info!(document_id = %document_id, version, "get_document_version invoked");
    let versions = state
        .get_document_versions(&document_id)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "document '{document_id}' version {version} not found"
            ))
        })?;

    let response = v1::documents::DocumentVersionRecord::new(
        document_id.clone(),
        entry.version,
        entry.timestamp,
        entry.item.into(),
    );
    info!(document_id = %document_id, version, "get_document_version completed");
    Ok(Json(response))
}

pub async fn get_document_by_path(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Result<Json<v1::documents::DocumentRecord>, ApiError> {
    info!(path = %path, "get_document_by_path invoked");
    let (document_id, document) = state
        .get_document_by_exact_path(&path)
        .await
        .map_err(|err| map_document_error_by_path(err, &path))?;

    let response = v1::documents::DocumentRecord::new(document_id.clone(), document.item.into());
    info!(document_id = %document_id, path = %path, "get_document_by_path completed");
    Ok(Json(response))
}

fn map_document_error(err: StoreError, document_id: Option<&DocumentId>) -> ApiError {
    match err {
        StoreError::DocumentNotFound(not_found_id) => {
            let id = document_id.unwrap_or(&not_found_id);
            error!(document_id = %id, "document not found");
            ApiError::not_found(format!("document '{id}' not found"))
        }
        other => {
            error!(error = %other, "document store error");
            ApiError::internal(anyhow!("document store error: {other}"))
        }
    }
}

fn map_document_error_by_path(err: StoreError, path: &str) -> ApiError {
    match err {
        StoreError::DocumentNotFoundAtPath(not_found_path) => {
            error!(path = %not_found_path, "document not found at path");
            ApiError::not_found(format!("document not found at path '{not_found_path}'"))
        }
        StoreError::DocumentNotFound(not_found_id) => {
            error!(document_id = %not_found_id, path = %path, "document not found");
            ApiError::not_found(format!("document not found at path '{path}'"))
        }
        other => {
            error!(error = %other, "document store error");
            ApiError::internal(anyhow!("document store error: {other}"))
        }
    }
}

fn map_upsert_document_error(err: UpsertDocumentError) -> ApiError {
    match err {
        UpsertDocumentError::JobNotFound { job_id, source } => {
            error!(job_id = %job_id, error = %source, "created_by job not found");
            ApiError::bad_request("created_by must reference a running job")
        }
        UpsertDocumentError::JobStatusLookup { job_id, source } => {
            error!(job_id = %job_id, error = %source, "failed to validate job status");
            ApiError::internal(anyhow!(
                "failed to validate job status for '{job_id}': {source}"
            ))
        }
        UpsertDocumentError::JobNotRunning { job_id, status } => {
            error!(job_id = %job_id, status = ?status, "created_by job not running");
            ApiError::bad_request("created_by must reference a running job")
        }
        UpsertDocumentError::DocumentNotFound {
            document_id,
            source,
        } => {
            error!(document_id = %document_id, error = %source, "document not found");
            ApiError::not_found(format!("document '{document_id}' not found"))
        }
        UpsertDocumentError::Store { source } => {
            error!(error = %source, "document store operation failed");
            ApiError::internal(anyhow!("document store operation failed: {source}"))
        }
    }
}
