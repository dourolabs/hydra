use crate::domain::actors::{Actor, ActorRef};
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
    DocumentId,
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
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for DocumentVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((document_id, version)) =
            Path::<(DocumentId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
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
    let (document_id, version) = state
        .upsert_document(None, payload.document.into(), ActorRef::from(&actor))
        .await
        .map_err(map_upsert_document_error)?;

    info!(actor = %actor.name(), document_id = %document_id, "create_document completed");
    Ok(Json(v1::documents::UpsertDocumentResponse::new(
        document_id,
        version,
    )))
}

pub async fn update_document(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    DocumentIdPath(document_id): DocumentIdPath,
    Json(payload): Json<v1::documents::UpsertDocumentRequest>,
) -> Result<Json<v1::documents::UpsertDocumentResponse>, ApiError> {
    info!(actor = %actor.name(), document_id = %document_id, "update_document invoked");
    let (document_id, version) = state
        .upsert_document(
            Some(document_id.clone()),
            payload.document.into(),
            ActorRef::from(&actor),
        )
        .await
        .map_err(map_upsert_document_error)?;

    info!(actor = %actor.name(), document_id = %document_id, "update_document completed");
    Ok(Json(v1::documents::UpsertDocumentResponse::new(
        document_id,
        version,
    )))
}

pub async fn get_document(
    State(state): State<AppState>,
    DocumentIdPath(document_id): DocumentIdPath,
    Query(query): Query<v1::documents::GetDocumentQuery>,
) -> Result<Json<v1::documents::DocumentVersionRecord>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    info!(document_id = %document_id, include_deleted, "get_document invoked");
    let document = state
        .get_document(&document_id, include_deleted)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let response = v1::documents::DocumentVersionRecord::new(
        document_id.clone(),
        document.version,
        document.timestamp,
        document.item.into(),
    );
    info!(document_id = %document_id, "get_document completed");
    Ok(Json(response))
}

pub async fn list_documents(
    State(state): State<AppState>,
    Query(query): Query<v1::documents::SearchDocumentsQuery>,
) -> Result<Json<v1::documents::ListDocumentsResponse>, ApiError> {
    info!(query = ?query.q, path_prefix = ?query.path_prefix, path_is_exact = ?query.path_is_exact, created_by = ?query.created_by, include_deleted = ?query.include_deleted, "list_documents invoked");
    let documents = state
        .list_documents(&query)
        .await
        .map_err(|err| map_document_error(err, None))?;

    let records = documents
        .into_iter()
        .map(|(id, versioned)| {
            v1::documents::DocumentVersionRecord::new(
                id,
                versioned.version,
                versioned.timestamp,
                versioned.item.into(),
            )
        })
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
        version: raw_version,
    }: DocumentVersionPath,
) -> Result<Json<v1::documents::DocumentVersionRecord>, ApiError> {
    info!(document_id = %document_id, raw_version = raw_version.as_i64(), "get_document_version invoked");
    let versions = state
        .get_document_versions(&document_id)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version =
        super::resolve_version(raw_version, max_version, "document", document_id.as_ref())?;

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
        UpsertDocumentError::PolicyViolation(violation) => ApiError::bad_request(violation.message),
    }
}

pub async fn delete_document(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    DocumentIdPath(document_id): DocumentIdPath,
) -> Result<Json<v1::documents::DocumentVersionRecord>, ApiError> {
    info!(document_id = %document_id, "delete_document invoked");
    state
        .delete_document(&document_id, ActorRef::from(&actor))
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    let document = state
        .get_document(&document_id, true)
        .await
        .map_err(|err| map_document_error(err, Some(&document_id)))?;

    info!(document_id = %document_id, "delete_document completed");
    let response = v1::documents::DocumentVersionRecord::new(
        document_id,
        document.version,
        document.timestamp,
        document.item.into(),
    );
    Ok(Json(response))
}
