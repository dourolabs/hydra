use super::labels::LabelSummary;
use super::serde_helpers::{deserialize_comma_separated, serialize_comma_separated};
use crate::{DocumentId, DocumentPath, SessionId, VersionNumber, actor_ref::ActorRef};
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
    pub created_by: Option<SessionId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Document {
    pub fn new(
        title: String,
        body_markdown: String,
        path: Option<String>,
        created_by: Option<SessionId>,
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
    /// Batch-fetch specific documents by ID (comma-separated, max 100).
    /// Intersected with other filters when provided.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub ids: Vec<DocumentId>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub path_is_exact: Option<bool>,
    #[serde(default)]
    pub created_by: Option<SessionId>,
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
    /// Filter by whether a path is set. `true` = only docs with a path, `false` = only docs without.
    #[serde(default)]
    pub has_path: Option<bool>,
}

impl SearchDocumentsQuery {
    pub fn new(
        q: Option<String>,
        path_prefix: Option<String>,
        path_is_exact: Option<bool>,
        created_by: Option<SessionId>,
        include_deleted: Option<bool>,
    ) -> Self {
        Self {
            ids: Vec::new(),
            q,
            path_prefix,
            path_is_exact,
            created_by,
            include_deleted,
            limit: None,
            cursor: None,
            count: None,
            has_path: None,
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
    pub created_by: Option<SessionId>,
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

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListDocumentPathsQuery {
    #[serde(default)]
    pub prefix: Option<String>,
    /// Multiple path prefixes (comma-separated). When supplied, the response is
    /// the union of per-prefix listings. Mutually exclusive with `prefix`.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub prefixes: Vec<String>,
}

/// Inline document reference attached to a `PathChildEntry` when the entry's
/// `full_path` matches a live (non-deleted) document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PathChildDocumentRef {
    pub document_id: DocumentId,
    /// Document title at the time of the lookup. May be an empty string if the
    /// document has no title set; the frontend has its own fallback.
    pub title: String,
}

impl PathChildDocumentRef {
    pub fn new(document_id: DocumentId, title: String) -> Self {
        Self { document_id, title }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PathChildEntry {
    pub name: String,
    pub full_path: String,
    pub child_count: u64,
    pub is_document: bool,
    /// Populated when `is_document=true` and a live document exists at this
    /// exact path. Always `None`/absent for pure folder entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<PathChildDocumentRef>,
}

impl PathChildEntry {
    pub fn new(
        name: String,
        full_path: String,
        child_count: u64,
        is_document: bool,
        document: Option<PathChildDocumentRef>,
    ) -> Self {
        Self {
            name,
            full_path,
            child_count,
            is_document,
            document,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListDocumentPathsResponse {
    pub children: Vec<PathChildEntry>,
}

impl ListDocumentPathsResponse {
    pub fn new(children: Vec<PathChildEntry>) -> Self {
        Self { children }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use std::collections::HashMap;

    #[test]
    fn document_new_accepts_all_fields() {
        let created_by = SessionId::new();
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
            ids: Vec::new(),
            q: Some("api".to_string()),
            path_prefix: Some("docs/".to_string()),
            path_is_exact: None,
            created_by: Some(SessionId::new()),
            include_deleted: None,
            limit: None,
            cursor: None,
            count: None,
            has_path: None,
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
    fn search_documents_query_serializes_ids() {
        let query = SearchDocumentsQuery {
            ids: vec![
                "d-abcd".parse::<DocumentId>().unwrap(),
                "d-efgh".parse::<DocumentId>().unwrap(),
            ],
            ..SearchDocumentsQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("ids").map(String::as_str), Some("d-abcd,d-efgh"));
    }

    #[test]
    fn search_documents_query_deserializes_ids() {
        let query: SearchDocumentsQuery =
            serde_urlencoded::from_str("ids=d-abcd%2Cd-efgh").unwrap();
        assert_eq!(query.ids.len(), 2);
        assert_eq!(query.ids[0].as_ref(), "d-abcd");
        assert_eq!(query.ids[1].as_ref(), "d-efgh");
    }

    #[test]
    fn search_documents_query_omits_empty_ids() {
        let query = SearchDocumentsQuery::default();
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!(
            !params.contains_key("ids"),
            "empty ids vec should be omitted from serialization"
        );
    }

    #[test]
    fn document_summary_excludes_body_markdown() {
        let doc = Document::new(
            "My Doc".to_string(),
            "# Heading\n\nLong markdown body...".to_string(),
            Some("docs/test.md".to_string()),
            Some(SessionId::new()),
            false,
        )
        .unwrap();
        let summary = DocumentSummary::from(&doc);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("body_markdown").is_none());
    }

    #[test]
    fn document_summary_maps_all_fields() {
        let created_by = SessionId::new();
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
    fn list_document_paths_query_serializes_single_prefix() {
        let query = ListDocumentPathsQuery {
            prefix: Some("/agents/".to_string()),
            prefixes: Vec::new(),
        };
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("prefix").map(String::as_str), Some("/agents/"));
        assert!(!params.contains_key("prefixes"));
    }

    #[test]
    fn list_document_paths_query_serializes_prefixes() {
        let query = ListDocumentPathsQuery {
            prefix: None,
            prefixes: vec!["/agents/".to_string(), "/repos/".to_string()],
        };
        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!(!params.contains_key("prefix"));
        assert_eq!(
            params.get("prefixes").map(String::as_str),
            Some("/agents/,/repos/"),
        );
    }

    #[test]
    fn list_document_paths_query_deserializes_prefixes() {
        let query: ListDocumentPathsQuery =
            serde_urlencoded::from_str("prefixes=%2Fagents%2F%2C%2Frepos%2F").unwrap();
        assert_eq!(query.prefix, None);
        assert_eq!(
            query.prefixes,
            vec!["/agents/".to_string(), "/repos/".to_string()]
        );
    }

    #[test]
    fn list_document_paths_query_defaults_are_empty() {
        let query = ListDocumentPathsQuery::default();
        let params = serialize_query_params(&query);
        assert!(params.is_empty());
    }

    #[test]
    fn path_child_entry_omits_absent_document() {
        let entry =
            PathChildEntry::new("agents".to_string(), "/agents".to_string(), 3, false, None);
        let value = serde_json::to_value(&entry).unwrap();
        assert!(value.get("document").is_none());
    }

    #[test]
    fn path_child_entry_includes_document_ref_when_present() {
        let entry = PathChildEntry::new(
            "notes.md".to_string(),
            "/agents/pm/notes.md".to_string(),
            1,
            true,
            Some(PathChildDocumentRef::new(
                "d-abcd".parse::<DocumentId>().unwrap(),
                "Notes".to_string(),
            )),
        );
        let value = serde_json::to_value(&entry).unwrap();
        let document = value
            .get("document")
            .expect("document ref should be serialized");
        assert_eq!(
            document.get("document_id").and_then(|v| v.as_str()),
            Some("d-abcd")
        );
        assert_eq!(
            document.get("title").and_then(|v| v.as_str()),
            Some("Notes")
        );
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
