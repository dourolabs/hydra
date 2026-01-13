use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::MetisId;
use metis_common::{issues::Issue, jobs::Bundle, patches::Patch};
use std::collections::HashMap;

mod memory_store;

pub use metis_common::task_status::{Status, TaskError, TaskStatusLog};

/// Represents a dependency edge between tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub id: MetisId,
    pub name: Option<String>,
}

/// Represents a task in the Metis system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// A spawn task that creates a new job.
    Spawn {
        program: String,
        params: Vec<String>,
        context: Bundle,
        image: String,
        env_vars: HashMap<String, String>,
    },
}

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Task not found: {0}")]
    TaskNotFound(MetisId),
    #[error("Issue not found: {0}")]
    IssueNotFound(MetisId),
    #[error("Patch not found: {0}")]
    PatchNotFound(MetisId),
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
    /// Adds a new issue to the store and assigns it a MetisId.
    async fn add_issue(&mut self, issue: Issue) -> Result<MetisId, StoreError>;

    /// Retrieves an issue by its MetisId.
    async fn get_issue(&self, id: &MetisId) -> Result<Issue, StoreError>;

    /// Updates an existing issue in the store.
    async fn update_issue(&mut self, id: &MetisId, issue: Issue) -> Result<(), StoreError>;

    /// Lists all issues in the store with their corresponding IDs.
    async fn list_issues(&self) -> Result<Vec<(MetisId, Issue)>, StoreError>;

    /// Adds a new patch to the store and assigns it a MetisId.
    async fn add_patch(&mut self, patch: Patch) -> Result<MetisId, StoreError>;

    /// Retrieves a patch by its MetisId.
    async fn get_patch(&self, id: &MetisId) -> Result<Patch, StoreError>;

    /// Updates an existing patch in the store.
    async fn update_patch(&mut self, id: &MetisId, patch: Patch) -> Result<(), StoreError>;

    /// Lists all patches in the store with their corresponding IDs.
    async fn list_patches(&self) -> Result<Vec<(MetisId, Patch)>, StoreError>;

    /// Lists all issues that declare the provided issue as a parent via `child-of`.
    async fn get_issue_children(&self, issue_id: &MetisId) -> Result<Vec<MetisId>, StoreError>;

    /// Lists all issues that are blocked on the provided issue.
    async fn get_issue_blocked_on(&self, issue_id: &MetisId) -> Result<Vec<MetisId>, StoreError>;

    /// Returns whether the issue is ready to be worked on based on its status and dependencies.
    async fn is_issue_ready(&self, issue_id: &MetisId) -> Result<bool, StoreError>;

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
    /// * `parent_edges` - A vector of dependency edges representing parent tasks that must complete first
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task(
        &mut self,
        task: Task,
        parent_edges: Vec<Edge>,
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
    /// * `parent_edges` - A vector of dependency edges representing parent tasks that must complete first
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task_with_id(
        &mut self,
        metis_id: MetisId,
        task: Task,
        parent_edges: Vec<Edge>,
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
    /// A vector of dependency edges for the parent tasks, or an error if the task doesn't exist
    async fn get_parents(&self, id: &MetisId) -> Result<Vec<Edge>, StoreError>;

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

    /// Gets the result of a task by its MetisId.
    ///
    /// # Arguments
    /// * `id` - The MetisId to look up
    ///
    /// # Returns
    /// Some(Ok(())) if the task completed successfully,
    /// Some(Err(TaskError)) if the task completed with an error,
    /// None if the task doesn't exist or has no result yet
    fn get_result(&self, id: &MetisId) -> Option<Result<(), TaskError>>;

    /// Records an emitted event for a running task.
    async fn emit_task_artifacts(
        &mut self,
        id: &MetisId,
        artifact_ids: Vec<MetisId>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError>;

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
    /// - From Running to Complete (if result is Ok)
    /// - From Running to Failed (if result is Err)
    ///
    /// This function will also update the status of dependent tasks:
    /// - All children (dependents) are checked
    /// - Children that are Blocked and have all their parents Complete or Failed are moved to Pending
    ///
    /// # Arguments
    /// * `id` - The MetisId of the task to update
    /// * `result` - The result of the task execution. If Ok, the task is marked as Complete.
    ///              If Err, the task is marked as Failed with the error as the failure reason.
    /// * `end_time` - The timestamp when the task completed or failed
    /// * `last_message` - Optional final worker message to store with the completion event
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The task is not in Running state
    async fn mark_task_complete(
        &mut self,
        id: &MetisId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}

pub use memory_store::MemoryStore;
