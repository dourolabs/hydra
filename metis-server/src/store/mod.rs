use async_trait::async_trait;
use crate::job_engine::MetisId;
use metis_common::{
    jobs::CreateJobRequestContext,
    job_outputs::JobOutputPayload,
};

mod memory_store;

/// Represents a task in the Metis system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// A spawn task that creates a new job.
    Spawn {
        prompt: String,
        context: CreateJobRequestContext,
        result: Option<JobOutputPayload>,
    },
    /// An ask task that queries the human user for information.
    Ask,
}

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Task not found: {0}")]
    TaskNotFound(MetisId),
    #[error("Invalid dependency: {0}")]
    InvalidDependency(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Trait for storing and managing a directed acyclic graph (DAG) of tasks.
///
/// The Store holds a DAG where:
/// - Vertices are tasks identified by MetisId
/// - Edges represent blocking dependencies (source must complete before destination can start)
/// - The graph must remain acyclic
#[async_trait]
pub trait Store: Send + Sync {
    /// Adds a task to the store with its parent dependencies.
    ///
    /// The parent tasks must complete before this task can start.
    /// This operation will fail if adding the dependencies would create a cycle
    /// or if any parent task doesn't exist.
    ///
    /// # Arguments
    /// * `task` - The task to add
    /// * `parent_ids` - A vector of MetisIds representing parent tasks that must complete first
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task already exists
    /// - Any parent task doesn't exist
    /// - Adding the dependencies would create a cycle
    async fn add_task(&mut self, task: Task, parent_ids: Vec<MetisId>) -> Result<MetisId, StoreError>;

    /// Gets a task by its MetisId.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// The task if found, or an error if not found
    async fn get_task(&self, id: &MetisId) -> Result<Task, StoreError>;

    /// Gets all parent tasks (dependencies) of a given task.
    ///
    /// Parents are tasks that must complete before the given task can start.
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task
    ///
    /// # Returns
    /// A vector of MetisIds representing the parent tasks, or an error if the task doesn't exist
    async fn get_parents(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError>;

    /// Gets all child tasks (dependents) of a given task.
    ///
    /// Children are tasks that must wait for the given task to complete before they can start.
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task
    ///
    /// # Returns
    /// A vector of MetisIds representing the child tasks, or an error if the task doesn't exist
    async fn get_children(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError>;

    /// Removes a task and all its associated edges from the store.
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to remove
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if the task doesn't exist
    async fn remove_task(&mut self, id: &MetisId) -> Result<(), StoreError>;

    /// Lists all task IDs in the store.
    ///
    /// # Returns
    /// A vector of all MetisIds in the store
    async fn list_tasks(&self) -> Result<Vec<MetisId>, StoreError>;
}

