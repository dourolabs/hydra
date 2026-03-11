use super::labels::LabelSummary;
use crate::{DocumentId, DocumentPath, TaskId, VersionNumber, actor_ref::ActorRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Document {
    #[serde(default)]
    pub title: String,
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<DocumentPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<TaskId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Document {
    pub fn new(
        title: String,
        body_markdown: String,
        path: Option<String>,
        created_by: Option<TaskId>,
        deleted: bool,
    ) -> Result<Self, crate::DocumentPathError> {
        let path = path.map(|p| p.parse()).transpose()?;
        Ok(Self {
            title,
            body_markdown,
            path,
            created_by,
            deleted,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DocumentVersionRecord {
    pub document_id: DocumentId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub document: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelSummary>,
}

impl DocumentVersionRecord {
    pub fn new(
        document_id: DocumentId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        document: Document,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
        labels: Vec<LabelSummary>,
    ) -> Self {
        Self {
            document_id,
            version,
            timestamp,
            document,
            actor,
            creation_time,
            labels,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
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
    /// Maximum number of results to return. When omitted, all results are returned.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
    /// When true, include `total_count` in the response.
    #[serde(default)]
    pub count: Option<bool>,
}

impl SearchDocumentsQuery {
    pub fn new(
        q: Option<String>,
        path_prefix: Option<String>,
        path_is_exact: Option<bool>,
        created_by: Option<TaskId>,
        include_deleted: Option<bool>,
    ) -> Self {
        Self {
            q,
            path_prefix,
            path_is_exact,
            created_by,
            include_deleted,
            limit: None,
            cursor: None,
            count: None,
        }
    }

    pub fn with_path_is_exact(mut self, path_is_exact: bool) -> Self {
        self.path_is_exact = Some(path_is_exact);
        self
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct GetDocumentQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertDocumentRequest {
    pub document: Document,
}

impl UpsertDocumentRequest {
    pub fn new(document: Document) -> Self {
        Self { document }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertDocumentResponse {
    pub document_id: DocumentId,
    pub version: VersionNumber,
}

impl UpsertDocumentResponse {
    pub fn new(document_id: DocumentId, version: VersionNumber) -> Self {
        Self {
            document_id,
            version,
        }
    }
}

/// Lightweight summary of a document for list views.
///
/// Excludes `body_markdown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DocumentSummary {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<DocumentPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<TaskId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelSummary>,
}

impl From<&Document> for DocumentSummary {
    fn from(doc: &Document) -> Self {
        DocumentSummary {
            title: doc.title.clone(),
            path: doc.path.clone(),
            created_by: doc.created_by.clone(),
            deleted: doc.deleted,
            labels: Vec::new(),
        }
    }
}

/// Summary-level version record for document list responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DocumentSummaryRecord {
    pub document_id: DocumentId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub document: DocumentSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
}

impl DocumentSummaryRecord {
    pub fn new(
        document_id: DocumentId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        document: DocumentSummary,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
        labels: Vec<LabelSummary>,
    ) -> Self {
        let mut document = document;
        document.labels = labels;
        Self {
            document_id,
            version,
            timestamp,
            document,
            actor,
            creation_time,
        }
    }
}

impl From<&DocumentVersionRecord> for DocumentSummaryRecord {
    fn from(record: &DocumentVersionRecord) -> Self {
        let mut document = DocumentSummary::from(&record.document);
        document.labels = record.labels.clone();
        DocumentSummaryRecord {
            document_id: record.document_id.clone(),
            version: record.version,
            timestamp: record.timestamp,
            document,
            actor: record.actor.clone(),
            creation_time: record.creation_time,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListDocumentsResponse {
    pub documents: Vec<DocumentSummaryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

impl ListDocumentsResponse {
    pub fn new(documents: Vec<DocumentSummaryRecord>) -> Self {
        Self {
            documents,
            next_cursor: None,
            total_count: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListDocumentVersionsResponse {
    pub versions: Vec<DocumentVersionRecord>,
}

impl ListDocumentVersionsResponse {
    pub fn new(versions: Vec<DocumentVersionRecord>) -> Self {
        Self { versions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use std::collections::HashMap;

    #[test]
    fn document_new_accepts_all_fields() {
        let created_by = TaskId::new();
        let document = Document::new(
            "Title".to_string(),
            "Body".to_string(),
            Some("docs/path.md".to_string()),
            Some(created_by.clone()),
            false,
        )
        .unwrap();
        assert_eq!(document.path.as_deref(), Some("/docs/path.md"));
        assert_eq!(document.created_by, Some(created_by));
    }

    #[test]
    fn search_documents_query_serializes() {
        let query = SearchDocumentsQuery {
            q: Some("api".to_string()),
            path_prefix: Some("docs/".to_string()),
            path_is_exact: None,
            created_by: Some(TaskId::new()),
            include_deleted: None,
            limit: None,
            cursor: None,
            count: None,
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("q").map(String::as_str), Some("api"));
        assert_eq!(params.get("path_prefix").map(String::as_str), Some("docs/"));
        assert!(params.contains_key("created_by"));
    }

    #[test]
    fn search_documents_query_omits_empty_fields() {
        let params = serialize_query_params(&SearchDocumentsQuery::default());
        assert!(params.is_empty());
    }

    #[test]
    fn search_documents_query_serializes_path_is_exact() {
        let query = SearchDocumentsQuery::new(
            None,
            Some("docs/file.md".to_string()),
            Some(true),
            None,
            None,
        );

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("path_prefix").map(String::as_str),
            Some("docs/file.md")
        );
        assert_eq!(
            params.get("path_is_exact").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn search_documents_query_deserializes_path_is_exact() {
        let json = r#"{"path_prefix": "docs/file.md", "path_is_exact": true}"#;
        let query: SearchDocumentsQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.path_prefix.as_deref(), Some("docs/file.md"));
        assert_eq!(query.path_is_exact, Some(true));
    }

    #[test]
    fn search_documents_query_defaults_path_is_exact_to_none() {
        let json = r#"{"path_prefix": "docs/"}"#;
        let query: SearchDocumentsQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.path_prefix.as_deref(), Some("docs/"));
        assert_eq!(query.path_is_exact, None);
    }

    #[test]
    fn document_summary_excludes_body_markdown() {
        let doc = Document::new(
            "My Doc".to_string(),
            "# Heading\n\nLong markdown body...".to_string(),
            Some("docs/test.md".to_string()),
            Some(TaskId::new()),
            false,
        )
        .unwrap();
        let summary = DocumentSummary::from(&doc);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("body_markdown").is_none());
    }

    #[test]
    fn document_summary_maps_all_fields() {
        let created_by = TaskId::new();
        let doc = Document::new(
            "Title".to_string(),
            "body".to_string(),
            Some("docs/path.md".to_string()),
            Some(created_by.clone()),
            false,
        )
        .unwrap();
        let summary = DocumentSummary::from(&doc);
        assert_eq!(summary.title, "Title");
        assert_eq!(summary.path.as_deref(), Some("/docs/path.md"));
        assert_eq!(summary.created_by, Some(created_by));
        assert!(!summary.deleted);
    }

    #[test]
    fn document_summary_record_from_version_record() {
        let doc =
            Document::new("Title".to_string(), "body".to_string(), None, None, false).unwrap();
        let doc_id = DocumentId::new();
        let ts = chrono::Utc::now();
        let record = DocumentVersionRecord::new(doc_id.clone(), 2, ts, doc, None, ts, Vec::new());
        let summary_record = DocumentSummaryRecord::from(&record);
        assert_eq!(summary_record.document_id, doc_id);
        assert_eq!(summary_record.version, 2);
        assert_eq!(summary_record.document.title, "Title");
        assert_eq!(summary_record.actor, None);
    }
}
