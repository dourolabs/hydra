use crate::job_engine::MetisId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{
    job_outputs::{JobOutputPayload, JobOutputType},
    jobs::CreateJobRequestContext,
};

mod memory_store;

/// Represents a task in the Metis system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// A spawn task that creates a new job.
    Spawn {
        prompt: String,
        context: CreateJobRequestContext,
        output_type: JobOutputType,
        result: Option<JobOutputPayload>,
    },
    /// An ask task that queries the human user for information.
    Ask,
}

#[derive(Debug, Clone)]
pub struct TaskStatusLog {
    pub creation_time: DateTime<Utc>,
    /// When the job started running
    pub start_time: Option<DateTime<Utc>>,
    /// When the job completed (succeeded or failed)
    pub end_time: Option<DateTime<Utc>>,
    /// Current status of the task
    pub current_status: Status,
    pub failure_reason: Option<String>,
}

/// Represents the status of a task in the Metis system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Task is blocked by dependencies that haven't completed yet.
    Blocked,
    /// Task is ready to run but hasn't started yet.
    Pending,
    /// Task is currently running.
    Running,
    /// Task has completed successfully.
    Complete,
    /// Task has failed.
    Failed,
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
    #[error("Invalid status transition: task is not in Pending state")]
    InvalidStatusTransition,
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
    ///
    /// # Arguments
    /// * `task` - The task to add
    /// * `parent_ids` - A vector of MetisIds representing parent tasks that must complete first
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task(
        &mut self,
        task: Task,
        parent_ids: Vec<MetisId>,
        creation_time: DateTime<Utc>,
    ) -> Result<MetisId, StoreError>;

    /// Adds a task to the store with a specific ID and parent dependencies.
    ///
    /// This is similar to `add_task`, but allows specifying the MetisId directly.
    /// Useful when the ID comes from an external source (e.g., Kubernetes job ID).
    ///
    /// # Arguments
    /// * `metis_id` - The MetisId to use for this task
    /// * `task` - The task to add
    /// * `parent_ids` - A vector of MetisIds representing parent tasks that must complete first
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task already exists
    /// - Any parent task doesn't exist
    /// - Adding the dependencies would create a cycle
    ///
    /// # Arguments
    /// * `metis_id` - The MetisId to use for this task
    /// * `task` - The task to add
    /// * `parent_ids` - A vector of MetisIds representing parent tasks that must complete first
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task_with_id(
        &mut self,
        metis_id: MetisId,
        task: Task,
        parent_ids: Vec<MetisId>,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Updates an existing task in the store.
    ///
    /// This function overwrites the task data for the given vertex without
    /// modifying the edge structure of the graph (parent and child relationships
    /// remain unchanged).
    ///
    /// # Arguments
    /// * `metis_id` - The MetisId of the task to update
    /// * `task` - The new Task to store for this vertex
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if the task doesn't exist
    async fn update_task(&mut self, metis_id: &MetisId, task: Task) -> Result<(), StoreError>;

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

    /// Lists all task IDs with the specified status in the store.
    ///
    /// # Arguments
    /// * `status` - The status to filter by
    ///
    /// # Returns
    /// A vector of MetisIds for tasks with the specified status
    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<MetisId>, StoreError>;

    /// Gets the status of a task by its MetisId.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// The status if found, or an error if not found
    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError>;

    /// Gets the status log for a task by its MetisId.
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

    /// Marks a task as running.
    ///
    /// Valid transitions:
    /// - From Pending to Running
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The task is not in Pending state
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    /// * `start_time` - The timestamp when the task started running
    async fn mark_task_running(
        &mut self,
        id: &MetisId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Marks a task as complete.
    ///
    /// Valid transitions:
    /// - From Running to Complete
    ///
    /// This function will also update the status of dependent tasks:
    /// - All children (dependents) are checked
    /// - Children that are Blocked and have all their parents Complete or Failed are moved to Pending
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The task is not in Running state
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    /// * `end_time` - The timestamp when the task completed
    async fn mark_task_complete(
        &mut self,
        id: &MetisId,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Marks a task as failed.
    ///
    /// Valid transitions:
    /// - From Running to Failed
    ///
    /// This function will also update the status of dependent tasks:
    /// - All children (dependents) are checked
    /// - Children that are Blocked and have all their parents Complete or Failed are moved to Pending
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The task is not in Running state
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    /// * `failure_reason` - The reason why the task failed
    /// * `end_time` - The timestamp when the task failed
    async fn mark_task_failed(
        &mut self,
        id: &MetisId,
        failure_reason: String,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}

pub use memory_store::MemoryStore;
