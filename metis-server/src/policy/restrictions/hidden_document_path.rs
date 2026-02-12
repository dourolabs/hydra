use async_trait::async_trait;
use metis_common::documents::has_hidden_segment;

use crate::policy::context::{OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Rejects document paths that contain hidden segments (components starting with `.`).
#[derive(Default)]
pub struct HiddenDocumentPathRestriction;

impl HiddenDocumentPathRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for HiddenDocumentPathRestriction {
    fn name(&self) -> &str {
        "hidden_document_path"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        let OperationPayload::Document { new, .. } = ctx.payload else {
            return Ok(());
        };

        if let Some(path) = &new.path {
            let normalized = path.strip_prefix('/').unwrap_or(path);
            if has_hidden_segment(normalized) {
                return Err(PolicyViolation {
                    policy_name: self.name().to_string(),
                    message: format!(
                        "Document path \"{path}\" contains a hidden segment. \
                         Paths starting with . are not allowed."
                    ),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::UserOrWorker;
    use crate::domain::documents::Document;
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::MemoryStore;

    fn test_actor() -> UserOrWorker {
        UserOrWorker::Username(Username::from("test-user"))
    }

    fn make_doc(path: Option<&str>) -> Document {
        Document {
            title: String::new(),
            body_markdown: String::new(),
            path: path.map(String::from),
            created_by: None,
            deleted: false,
        }
    }

    #[tokio::test]
    async fn allows_normal_path() {
        let restriction = HiddenDocumentPathRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(Some("designs/policy-engine.md")),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_hidden_segment() {
        let restriction = HiddenDocumentPathRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(Some(".hidden/file.md")),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "hidden_document_path");
        assert!(violation.message.contains("hidden segment"));
        assert!(violation.message.contains(".hidden/file.md"));
    }

    #[tokio::test]
    async fn allows_no_path() {
        let restriction = HiddenDocumentPathRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(None),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
