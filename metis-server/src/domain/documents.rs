use metis_common::api::v1 as api;
use metis_common::{DocumentPath, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub title: String,
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<DocumentPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<SessionId>,
    #[serde(default)]
    pub deleted: bool,
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
        // Path is already a valid DocumentPath, so re-parsing via new() cannot fail.
        api::documents::Document::new(
            value.title,
            value.body_markdown,
            value.path.map(|p| p.to_string()),
            value.created_by,
            value.deleted,
        )
        .expect("domain Document always has a valid path")
    }
}
