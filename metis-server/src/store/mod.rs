use crate::domain::{
    actors::{Actor, ActorError},
    issues::{Issue, IssueGraphFilter},
    patches::Patch,
    users::{User, Username},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, PatchId, RepoName, TaskId, repositories::ServiceRepositoryConfig};
use std::collections::HashSet;

mod issue_graph;
mod memory_store;
pub mod postgres;

pub use crate::domain::jobs::Task;
pub use crate::domain::task_status::{Status, TaskError, TaskStatusLog};

pub(crate) fn validate_actor_name(name: &str) -> Result<(), StoreError> {
    match Actor::parse_name(name) {
        Ok(_) => Ok(()),
        Err(ActorError::InvalidActorName(name)) => Err(StoreError::InvalidActorName(name)),
        Err(ActorError::GithubLookupFailed(message)) => Err(StoreError::Internal(message)),
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
    #[error("Repository not found: {0}")]
    RepositoryNotFound(RepoName),
    #[error("Repository already exists: {0}")]
    RepositoryAlreadyExists(RepoName),
    #[error("User not found: {0}")]
    UserNotFound(Username),
    #[error("User already exists: {0}")]
    UserAlreadyExists(Username),
    #[error("User not found for token")]
    UserNotFoundForToken,
    #[error("Actor not found: {0}")]
    ActorNotFound(String),
    #[error("Actor already exists: {0}")]
    ActorAlreadyExists(String),
    #[error("Invalid actor name: {0}")]
    InvalidActorName(String),
}

/// Trait for storing issues, patches, and tasks along with their statuses.
///
/// Implementations focus on persistence and referential integrity; application-specific
/// state transition rules (such as issue lifecycle validation) must be enforced by the
/// caller before invoking store operations.
#[async_trait]
pub trait Store: Send + Sync {
    /// Adds a repository configuration under the provided name.
    ///
    /// Returns an error if a repository with the same name already exists.
    async fn add_repository(
        &mut self,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<(), StoreError>;

    /// Retrieves a repository configuration by name.
    async fn get_repository(&self, name: &RepoName) -> Result<ServiceRepositoryConfig, StoreError>;

    /// Updates an existing repository configuration.
    ///
    /// Returns an error if the repository does not exist.
    async fn update_repository(
        &mut self,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<(), StoreError>;

    /// Lists all repository configurations keyed by name.
    async fn list_repositories(
        &self,
    ) -> Result<Vec<(RepoName, ServiceRepositoryConfig)>, StoreError>;

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

    /// Applies dependency graph filters and returns the matching issue IDs.
    ///
    /// Filters are intersected, and any filter referencing a missing issue
    /// should return `StoreError::IssueNotFound`.
    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError>;

    /// Adds a new patch to the store and assigns it a PatchId.
    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError>;

    /// Retrieves a patch by its PatchId.
    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError>;

    /// Updates an existing patch in the store.
    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError>;

    /// Lists all patches in the store with their corresponding IDs.
    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError>;

    /// Lists all issues that reference the provided patch ID.
    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all issues that declare the provided issue as a parent via `child-of`.
    #[allow(dead_code)]
    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all issues that are blocked on the provided issue.
    #[allow(dead_code)]
    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all task IDs spawned from the provided issue.
    #[allow(dead_code)]
    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError>;

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

    /// Adds a new actor to the store.
    async fn add_actor(&mut self, actor: Actor) -> Result<(), StoreError>;

    /// Gets an actor by its canonical name.
    async fn get_actor(&self, name: &str) -> Result<Actor, StoreError>;

    /// Lists all actors with their canonical names.
    async fn list_actors(&self) -> Result<Vec<(String, Actor)>, StoreError>;

    /// Adds a new user to the store.
    async fn add_user(&mut self, user: User) -> Result<(), StoreError>;

    /// Lists all users in the store.
    async fn list_users(&self) -> Result<Vec<User>, StoreError>;

    /// Deletes a user from the store.
    async fn delete_user(&mut self, username: &Username) -> Result<(), StoreError>;

    /// Updates the GitHub token for the requested user.
    async fn set_user_github_token(
        &mut self,
        username: &Username,
        github_token: String,
        github_user_id: Option<u64>,
    ) -> Result<User, StoreError>;

    /// Resolves a user by their GitHub token.
    async fn get_user_by_github_token(&self, github_token: &str) -> Result<User, StoreError>;

    /// Gets a user by their username.
    async fn get_user(&self, username: &Username) -> Result<User, StoreError>;
}

pub use memory_store::MemoryStore;
