use metis_common::TaskId;
use metis_common::api::v1 as api;
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
