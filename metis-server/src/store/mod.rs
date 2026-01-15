use crate::app::{BundleResolutionError, ResolvedBundle, ServiceState};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::constants::ENV_GH_TOKEN;
use metis_common::{IssueId, MetisId, PatchId, TaskId};
use metis_common::{issues::Issue, patches::Patch};
use std::collections::HashMap;

mod memory_store;

pub use metis_common::jobs::Task;
pub use metis_common::task_status::{Status, TaskError, TaskStatusLog};

#[derive(Debug, Clone)]
pub struct ResolvedTask {
    #[allow(dead_code)]
    pub context: ResolvedBundle,
    pub image: String,
    pub env_vars: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskResolutionError {
    #[error(transparent)]
    Bundle(#[from] BundleResolutionError),
    #[error("image must not be empty")]
    EmptyImage,
    #[error("default worker image must not be empty")]
    MissingDefaultImage,
}

pub trait TaskExt {
    fn resolve_context(
        &self,
        service_state: &ServiceState,
    ) -> Result<ResolvedBundle, BundleResolutionError>;

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError>;

    fn resolve_env_vars(&self, resolved: &ResolvedBundle) -> HashMap<String, String>;

    fn resolve(
        &self,
        service_state: &ServiceState,
        fallback_image: &str,
    ) -> Result<ResolvedTask, TaskResolutionError>;
}

impl TaskExt for Task {
    fn resolve_context(
        &self,
        service_state: &ServiceState,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        service_state.resolve_bundle_spec(self.context.clone())
    }

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError> {
        if let Some(image) = &self.image {
            let trimmed = image.trim();
            if trimmed.is_empty() {
                return Err(TaskResolutionError::EmptyImage);
            }
            return Ok(trimmed.to_string());
        }

        if let Some(default_image) = &resolved.default_image {
            let trimmed = default_image.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }

        let trimmed = fallback_image.trim();
        if trimmed.is_empty() {
            return Err(TaskResolutionError::MissingDefaultImage);
        }

        Ok(trimmed.to_string())
    }

    fn resolve_env_vars(&self, resolved: &ResolvedBundle) -> HashMap<String, String> {
        let mut env_vars = self.env_vars.clone();
        if let Some(token) = &resolved.github_token {
            env_vars
                .entry(ENV_GH_TOKEN.to_string())
                .or_insert_with(|| token.clone());
        }
        env_vars
    }

    fn resolve(
        &self,
        service_state: &ServiceState,
        fallback_image: &str,
    ) -> Result<ResolvedTask, TaskResolutionError> {
        let context = self.resolve_context(service_state)?;
        let image = self.resolve_image(&context, fallback_image)?;
        let env_vars = self.resolve_env_vars(&context);

        Ok(ResolvedTask {
            context,
            image,
            env_vars,
        })
    }
}

/// Error type for store operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Task not found: {0}")]
    TaskNotFound(TaskId),
    #[error("Issue not found: {0}")]
    IssueNotFound(IssueId),
    #[error("Patch not found: {0}")]
    PatchNotFound(PatchId),
    #[allow(dead_code)]
    #[error("Invalid dependency: {0}")]
    InvalidDependency(IssueId),
    #[error("Invalid issue status: {0}")]
    InvalidIssueStatus(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Invalid status transition: task is not in Pending state")]
    InvalidStatusTransition,
}

/// Trait for storing issues, patches, and tasks along with their statuses.
///
/// Implementations must enforce issue lifecycle invariants: an issue cannot be
/// closed while any blockers remain open or while it has open child issues.
/// Violations should return `StoreError::InvalidIssueStatus`.
#[async_trait]
pub trait Store: Send + Sync {
    /// Adds a new issue to the store and assigns it an IssueId.
    ///
    /// Returns an error if any declared dependencies reference missing issues.
    async fn add_issue(&mut self, issue: Issue) -> Result<IssueId, StoreError>;

    /// Retrieves an issue by its IssueId.
    async fn get_issue(&self, id: &IssueId) -> Result<Issue, StoreError>;

    /// Updates an existing issue in the store.
    ///
    /// Returns an error if the issue does not exist or if any dependencies
    /// reference missing issues.
    async fn update_issue(&mut self, id: &IssueId, issue: Issue) -> Result<(), StoreError>;

    /// Lists all issues in the store with their corresponding IDs.
    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError>;

    /// Adds a new patch to the store and assigns it a PatchId.
    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError>;

    /// Retrieves a patch by its PatchId.
    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError>;

    /// Updates an existing patch in the store.
    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError>;

    /// Lists all patches in the store with their corresponding IDs.
    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError>;

    /// Lists all issues that declare the provided issue as a parent via `child-of`.
    #[allow(dead_code)]
    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all issues that are blocked on the provided issue.
    #[allow(dead_code)]
    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Returns whether the issue is ready to be worked on based on its status and dependencies.
    async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError>;

    /// Lists all active (pending or running) task IDs spawned from the provided issue.
    #[allow(dead_code)]
    async fn get_active_tasks_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<TaskId>, StoreError>;

    /// Adds a task to the store.
    ///
    /// Tasks start in the Pending state.
    /// # Arguments
    /// * `task` - The task to add
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task(
        &mut self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError>;

    /// Adds a task to the store with a specific ID.
    ///
    /// This is similar to `add_task`, but allows specifying the TaskId directly.
    /// Useful when the ID comes from an external source (e.g., Kubernetes job ID).
    ///
    /// # Arguments
    /// * `metis_id` - The TaskId to use for this task
    /// * `task` - The task to add
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task already exists
    ///
    /// # Arguments
    /// * `metis_id` - The TaskId to use for this task
    /// * `task` - The task to add
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task_with_id(
        &mut self,
        metis_id: TaskId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Updates an existing task in the store.
    ///
    /// This function overwrites the task data for the given vertex.
    ///
    /// # Arguments
    /// * `metis_id` - The TaskId of the task to update
    /// * `task` - The new Task to store for this vertex
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if the task doesn't exist
    #[allow(dead_code)]
    async fn update_task(&mut self, metis_id: &TaskId, task: Task) -> Result<(), StoreError>;

    /// Gets a task by its TaskId.
    ///
    /// # Arguments
    /// * `id` - The TaskId to look up
    ///
    /// # Returns
    /// The task if found, or an error if not found
    async fn get_task(&self, id: &TaskId) -> Result<Task, StoreError>;

    /// Lists all task IDs in the store.
    ///
    /// # Returns
    /// A vector of all TaskIds in the store
    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError>;

    /// Lists all task IDs with the specified status in the store.
    ///
    /// # Arguments
    /// * `status` - The status to filter by
    ///
    /// # Returns
    /// A vector of TaskIds for tasks with the specified status
    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError>;

    /// Gets the status of a task by its TaskId.
    ///
    /// # Arguments
    /// * `id` - The TaskId to look up
    ///
    /// # Returns
    /// The status if found, or an error if not found
    async fn get_status(&self, id: &TaskId) -> Result<Status, StoreError>;

    /// Gets the status log for a task by its TaskId.
    ///
    /// The status log contains timing information about the task's lifecycle,
    /// including when it was created, when it started running, when it completed,
    /// and any failure reason if applicable.
    ///
    /// # Arguments
    /// * `id` - The TaskId to look up
    ///
    /// # Returns
    /// The TaskStatusLog if found, or an error if not found
    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError>;

    /// Gets the result of a task by its TaskId.
    ///
    /// # Arguments
    /// * `id` - The TaskId to look up
    ///
    /// # Returns
    /// Some(Ok(())) if the task completed successfully,
    /// Some(Err(TaskError)) if the task completed with an error,
    /// None if the task doesn't exist or has no result yet
    #[allow(dead_code)]
    fn get_result(&self, id: &TaskId) -> Option<Result<(), TaskError>>;

    /// Records an emitted event for a running task.
    async fn emit_task_artifacts(
        &mut self,
        id: &TaskId,
        artifact_ids: Vec<MetisId>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Marks a task as running.
    ///
    /// Valid transitions:
    /// - From Pending to Running
    ///
    /// # Arguments
    /// * `id` - The TaskId of the task to update
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if:
    /// - The task doesn't exist
    /// - The task is not in Pending state
    ///
    /// # Arguments
    /// * `id` - The TaskId of the task to update
    /// * `start_time` - The timestamp when the task started running
    async fn mark_task_running(
        &mut self,
        id: &TaskId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Marks a task as complete.
    ///
    /// Valid transitions:
    /// - From Running to Complete (if result is Ok)
    /// - From Running to Failed (if result is Err)
    ///
    /// # Arguments
    /// * `id` - The TaskId of the task to update
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
        id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}

pub use memory_store::MemoryStore;
