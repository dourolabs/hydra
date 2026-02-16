use crate::{DocumentId, DocumentPath, TaskId, VersionNumber, actor_ref::ActorRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn new(title: String, body_markdown: String, deleted: bool) -> Self {
        Self {
            title,
            body_markdown,
            path: None,
            created_by: None,
            deleted,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Result<Self, crate::DocumentPathError> {
        let path_str = path.into();
        self.path = Some(path_str.parse()?);
        Ok(self)
    }

    pub fn with_created_by(mut self, created_by: TaskId) -> Self {
        self.created_by = Some(created_by);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DocumentVersionRecord {
    pub document_id: DocumentId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub document: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl DocumentVersionRecord {
    pub fn new(
        document_id: DocumentId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        document: Document,
        actor: Option<ActorRef>,
    ) -> Self {
        Self {
            document_id,
            version,
            timestamp,
            document,
            actor,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        }
    }

    pub fn with_path_is_exact(mut self, path_is_exact: bool) -> Self {
        self.path_is_exact = Some(path_is_exact);
        self
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDocumentQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListDocumentsResponse {
    pub documents: Vec<DocumentVersionRecord>,
}

impl ListDocumentsResponse {
    pub fn new(documents: Vec<DocumentVersionRecord>) -> Self {
        Self { documents }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    fn document_builder_supports_option_fields() {
        let created_by = TaskId::new();
        let document = Document::new("Title".to_string(), "Body".to_string(), false)
            .with_path("docs/path.md")
            .unwrap()
            .with_created_by(created_by.clone());
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
}
