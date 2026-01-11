use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::MetisId;
use metis_common::artifacts::{Artifact, ArtifactKind};

mod memory_store;

pub use metis_common::task_status::{Event, Status, TaskError, TaskStatusLog};

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Task not found: {0}")]
    TaskNotFound(MetisId),
    #[error("Artifact not found: {0}")]
    ArtifactNotFound(MetisId),
    #[error("Invalid dependency: {0}")]
    InvalidDependency(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Invalid status transition: task is not in Pending state")]
    InvalidStatusTransition,
}

/// Trait for storing artifacts and tracking status logs.
#[async_trait]
pub trait Store: Send + Sync {
    /// Adds a new artifact to the store and assigns it a MetisId.
    ///
    /// # Arguments
    /// * `artifact` - The artifact to store
    ///
    /// # Returns
    /// The generated MetisId for the artifact
    async fn add_artifact(&mut self, artifact: Artifact) -> Result<MetisId, StoreError>;

    /// Retrieves an artifact by its MetisId.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// The artifact if found, or an error if not found
    async fn get_artifact(&self, id: &MetisId) -> Result<Artifact, StoreError>;

    /// Updates an existing artifact in the store.
    ///
    /// # Arguments
    /// * `id` - The MetisId of the artifact to update
    /// * `artifact` - The new artifact value
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if the artifact doesn't exist
    async fn update_artifact(&mut self, id: &MetisId, artifact: Artifact)
    -> Result<(), StoreError>;

    /// Lists all artifacts in the store with their corresponding IDs.
    ///
    /// # Returns
    /// A vector of (MetisId, Artifact) tuples representing all stored artifacts
    async fn list_artifacts(&self) -> Result<Vec<(MetisId, Artifact)>, StoreError>;

    /// Lists artifacts of a specific type.
    async fn list_artifacts_with_type(
        &self,
        artifact_type: ArtifactKind,
    ) -> Result<Vec<(MetisId, Artifact)>, StoreError>;

    /// Lists artifacts of a specific type and status.
    async fn list_artifacts_with_type_and_status(
        &self,
        artifact_type: ArtifactKind,
        status: Status,
    ) -> Result<Vec<(MetisId, Artifact)>, StoreError>;

    /// Adds an artifact to the store with a specific ID.
    ///
    /// The artifact's status log will be initialized at the provided creation_time
    /// (sessions start in Pending, other artifacts default to Complete).
    async fn add_artifact_with_id(
        &mut self,
        metis_id: MetisId,
        artifact: Artifact,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Gets the status of an artifact by its MetisId.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// The status if found, or an error if not found
    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError>;

    /// Gets the status log for an artifact by its MetisId.
    ///
    /// The status log contains timing information about the task's lifecycle,
    /// including when it was created, when it started running, when it completed,
    /// and any failure reason if applicable.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// The TaskStatusLog if found, or an error if not found
    async fn get_status_log(&self, id: &MetisId) -> Result<TaskStatusLog, StoreError>;

    /// Appends a new status event for the task.
    ///
    /// Valid transitions:
    /// - From Pending to Running (with Event::Started)
    /// - From Running to Complete/Failed (with Event::Completed/Event::Failed)
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    /// * `event` - The status event to append
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The event represents an invalid status transition
    async fn append_status_event(&mut self, id: &MetisId, event: Event) -> Result<(), StoreError>;
}

pub use memory_store::MemoryStore;
