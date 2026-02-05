use chrono::{DateTime, Utc};
use metis_common::api::v1 as api;
use metis_common::{DocumentId, TaskId, VersionNumber};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub title: String,
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<TaskId>,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRecord {
    pub id: DocumentId,
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentVersionRecord {
    pub document_id: DocumentId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub document: Document,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDocumentsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub path_is_exact: Option<bool>,
    #[serde(default)]
    pub created_by: Option<TaskId>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertDocumentRequest {
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertDocumentResponse {
    pub document_id: DocumentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDocumentsResponse {
    pub documents: Vec<DocumentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDocumentVersionsResponse {
    pub versions: Vec<DocumentVersionRecord>,
}

impl From<api::documents::Document> for Document {
    fn from(value: api::documents::Document) -> Self {
        Document {
            title: value.title,
            body_markdown: value.body_markdown,
            path: value.path,
            created_by: value.created_by,
            deleted: value.deleted,
        }
    }
}

impl From<Document> for api::documents::Document {
    fn from(value: Document) -> Self {
        let mut document =
            api::documents::Document::new(value.title, value.body_markdown, value.deleted);
        document.path = value.path;
        document.created_by = value.created_by;
        document
    }
}

impl From<api::documents::DocumentRecord> for DocumentRecord {
    fn from(value: api::documents::DocumentRecord) -> Self {
        DocumentRecord {
            id: value.id,
            document: value.document.into(),
        }
    }
}

impl From<DocumentRecord> for api::documents::DocumentRecord {
    fn from(value: DocumentRecord) -> Self {
        api::documents::DocumentRecord::new(value.id, value.document.into())
    }
}

impl From<api::documents::DocumentVersionRecord> for DocumentVersionRecord {
    fn from(value: api::documents::DocumentVersionRecord) -> Self {
        DocumentVersionRecord {
            document_id: value.document_id,
            version: value.version,
            timestamp: value.timestamp,
            document: value.document.into(),
        }
    }
}

impl From<DocumentVersionRecord> for api::documents::DocumentVersionRecord {
    fn from(value: DocumentVersionRecord) -> Self {
        api::documents::DocumentVersionRecord::new(
            value.document_id,
            value.version,
            value.timestamp,
            value.document.into(),
        )
    }
}

impl From<api::documents::SearchDocumentsQuery> for SearchDocumentsQuery {
    fn from(value: api::documents::SearchDocumentsQuery) -> Self {
        SearchDocumentsQuery {
            q: value.q,
            path_prefix: value.path_prefix,
            path_is_exact: value.path_is_exact,
            created_by: value.created_by,
            include_deleted: value.include_deleted,
        }
    }
}

impl From<SearchDocumentsQuery> for api::documents::SearchDocumentsQuery {
    fn from(value: SearchDocumentsQuery) -> Self {
        api::documents::SearchDocumentsQuery::new(
            value.q,
            value.path_prefix,
            value.path_is_exact,
            value.created_by,
            value.include_deleted,
        )
    }
}

impl From<api::documents::UpsertDocumentRequest> for UpsertDocumentRequest {
    fn from(value: api::documents::UpsertDocumentRequest) -> Self {
        UpsertDocumentRequest {
            document: value.document.into(),
        }
    }
}

impl From<UpsertDocumentRequest> for api::documents::UpsertDocumentRequest {
    fn from(value: UpsertDocumentRequest) -> Self {
        api::documents::UpsertDocumentRequest::new(value.document.into())
    }
}

impl From<api::documents::UpsertDocumentResponse> for UpsertDocumentResponse {
    fn from(value: api::documents::UpsertDocumentResponse) -> Self {
        UpsertDocumentResponse {
            document_id: value.document_id,
        }
    }
}

impl From<UpsertDocumentResponse> for api::documents::UpsertDocumentResponse {
    fn from(value: UpsertDocumentResponse) -> Self {
        api::documents::UpsertDocumentResponse::new(value.document_id)
    }
}

impl From<api::documents::ListDocumentsResponse> for ListDocumentsResponse {
    fn from(value: api::documents::ListDocumentsResponse) -> Self {
        ListDocumentsResponse {
            documents: value.documents.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ListDocumentsResponse> for api::documents::ListDocumentsResponse {
    fn from(value: ListDocumentsResponse) -> Self {
        api::documents::ListDocumentsResponse::new(
            value.documents.into_iter().map(Into::into).collect(),
        )
    }
}

impl From<api::documents::ListDocumentVersionsResponse> for ListDocumentVersionsResponse {
    fn from(value: api::documents::ListDocumentVersionsResponse) -> Self {
        ListDocumentVersionsResponse {
            versions: value.versions.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ListDocumentVersionsResponse> for api::documents::ListDocumentVersionsResponse {
    fn from(value: ListDocumentVersionsResponse) -> Self {
        api::documents::ListDocumentVersionsResponse::new(
            value.versions.into_iter().map(Into::into).collect(),
        )
    }
}
