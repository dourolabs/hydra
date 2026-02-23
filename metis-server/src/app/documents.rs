use crate::{
    domain::{actors::ActorRef, documents::Document},
    store::{ReadOnlyStore, Status, StoreError},
};
use metis_common::{
    DocumentId, TaskId, VersionNumber, Versioned, api::v1::documents::SearchDocumentsQuery,
};
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum UpsertDocumentError {
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("created_by must reference a running job")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("document '{document_id}' not found")]
    DocumentNotFound {
        #[source]
        source: StoreError,
        document_id: DocumentId,
    },
    #[error("document store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
}

impl AppState {
    pub async fn upsert_document(
        &self,
        document_id: Option<DocumentId>,
        document: Document,
        actor: ActorRef,
    ) -> Result<(DocumentId, VersionNumber), UpsertDocumentError> {
        let store = self.store.as_ref();

        let old_document = match &document_id {
            Some(id) => {
                let existing =
                    store
                        .get_document(id, false)
                        .await
                        .map_err(|source| match source {
                            StoreError::DocumentNotFound(_) => {
                                UpsertDocumentError::DocumentNotFound {
                                    document_id: id.clone(),
                                    source,
                                }
                            }
                            other => UpsertDocumentError::Store { source: other },
                        })?;
                Some(existing.item)
            }
            None => None,
        };

        // Run restriction policies before persisting
        match &document_id {
            Some(id) => {
                self.policy_engine
                    .check_update_document(id, &document, old_document.as_ref(), store, &actor)
                    .await?;
            }
            None => {
                self.policy_engine
                    .check_create_document(&document, store, &actor)
                    .await?;
            }
        }

        match document_id {
            Some(id) => {
                let mut document = document;
                // old_document is Some in update path
                document.created_by = old_document.unwrap().created_by;

                let version = self
                    .store
                    .update_document_with_actor(&id, document, actor)
                    .await
                    .map_err(|source| match source {
                        StoreError::DocumentNotFound(_) => UpsertDocumentError::DocumentNotFound {
                            document_id: id.clone(),
                            source,
                        },
                        other => UpsertDocumentError::Store { source: other },
                    })?;

                info!(document_id = %id, "document updated");
                Ok((id, version))
            }
            None => {
                let created_by = document.created_by.clone();
                let (document_id, version) = self
                    .store
                    .add_document_with_actor(document, actor)
                    .await
                    .map_err(|source| UpsertDocumentError::Store { source })?;

                info!(
                    document_id = %document_id,
                    created_by = ?created_by,
                    "document created"
                );
                Ok((document_id, version))
            }
        }
    }

    pub async fn get_document(
        &self,
        document_id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let store = self.store.as_ref();
        store.get_document(document_id, include_deleted).await
    }

    pub async fn get_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let store = self.store.as_ref();
        store.get_document_versions(document_id).await
    }

    pub async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_documents(query).await
    }

    pub async fn list_documents_paginated(
        &self,
        query: &SearchDocumentsQuery,
        pagination: &metis_common::api::v1::pagination::PaginationParams,
    ) -> Result<crate::store::PaginatedResult<(DocumentId, Versioned<Document>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_documents_paginated(query, pagination).await
    }

    pub async fn delete_document(
        &self,
        document_id: &DocumentId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store
            .delete_document_with_actor(document_id, actor)
            .await?;
        Ok(())
    }

    pub async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let store = self.store.as_ref();
        store.get_documents_by_path(path_prefix).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{domain::actors::ActorRef, domain::documents::Document, test_utils::test_state};

    #[tokio::test]
    async fn upsert_document_allows_normal_path() {
        let state = test_state();
        let document = Document {
            title: "Test".to_string(),
            body_markdown: "body".to_string(),
            path: Some("docs/notes.md".parse().unwrap()),
            created_by: None,
            deleted: false,
        };

        let result = state
            .upsert_document(None, document, ActorRef::test())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn upsert_document_allows_no_path() {
        let state = test_state();
        let document = Document {
            title: "Test".to_string(),
            body_markdown: "body".to_string(),
            path: None,
            created_by: None,
            deleted: false,
        };

        let result = state
            .upsert_document(None, document, ActorRef::test())
            .await;
        assert!(result.is_ok());
    }
}
