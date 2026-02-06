use crate::domain::{
    actors::{Actor, ActorError},
    documents::Document,
    issues::{Issue, IssueGraphFilter},
    patches::Patch,
    task_status::Event,
    users::{User, Username},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, PatchId, RepoName, TaskId, Versioned,
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::HashSet;

mod issue_graph;
mod memory_store;
pub mod migration;
pub mod postgres;
pub mod postgres_v2;

pub use crate::domain::jobs::Task;
pub use crate::domain::task_status::{Status, TaskError, TaskStatusLog};

pub(crate) fn validate_actor_name(name: &str) -> Result<(), StoreError> {
    match Actor::parse_name(name) {
        Ok(_) => Ok(()),
        Err(ActorError::InvalidActorName(name)) => Err(StoreError::InvalidActorName(name)),
    }
}

pub(crate) fn task_status_log_from_versions(versions: &[Versioned<Task>]) -> Option<TaskStatusLog> {
    let (first, rest) = versions.split_first()?;
    let mut log = TaskStatusLog::new(first.item.status, first.timestamp);
    let mut last_status = first.item.status;

    for entry in rest {
        let status = entry.item.status;
        if status == last_status {
            continue;
        }

        let event = match status {
            Status::Created => Event::Created {
                at: entry.timestamp,
                status,
            },
            Status::Pending => Event::Created {
                at: entry.timestamp,
                status,
            },
            Status::Running => Event::Started {
                at: entry.timestamp,
            },
            Status::Complete => Event::Completed {
                at: entry.timestamp,
                last_message: entry.item.last_message.clone(),
            },
            Status::Failed => Event::Failed {
                at: entry.timestamp,
                error: entry
                    .item
                    .error
                    .clone()
                    .unwrap_or(TaskError::JobEngineError {
                        reason: "missing failure reason".to_string(),
                    }),
            },
        };

        log.events.push(event);
        last_status = status;
    }

    Some(log)
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
    #[error("Document not found: {0}")]
    DocumentNotFound(DocumentId),
    #[allow(dead_code)]
    #[error("Invalid dependency: {0}")]
    InvalidDependency(IssueId),
    #[error("Invalid issue status: {0}")]
    InvalidIssueStatus(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Invalid status transition for task")]
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
    #[error("Invalid GitHub token: {0}")]
    GithubTokenInvalid(String),
    #[error("Invalid actor name: {0}")]
    InvalidActorName(String),
    #[error("Invalid auth token")]
    InvalidAuthToken,
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
    async fn add_repository(&self, name: RepoName, config: Repository) -> Result<(), StoreError>;

    /// Retrieves a repository configuration by name.
    ///
    /// # Arguments
    /// * `name` - The RepoName to look up
    /// * `include_deleted` - If true, returns the repository even if it has been soft-deleted.
    ///   If false, returns `StoreError::RepositoryNotFound` for deleted repositories.
    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError>;

    /// Updates an existing repository configuration.
    ///
    /// Returns an error if the repository does not exist.
    async fn update_repository(&self, name: RepoName, config: Repository)
    -> Result<(), StoreError>;

    /// Lists repository configurations keyed by name.
    ///
    /// By default, deleted repositories are filtered out unless `include_deleted: true`
    /// is set in the query.
    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError>;

    /// Soft-deletes a repository by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the repository with `deleted: true`.
    /// The repository can still be retrieved via `get_repository` but will be filtered
    /// from `list_repositories` by default.
    async fn delete_repository(&self, name: &RepoName) -> Result<(), StoreError>;

    /// Adds a new issue to the store and assigns it an IssueId.
    ///
    /// Returns an error if any declared dependencies reference missing issues.
    async fn add_issue(&self, issue: Issue) -> Result<IssueId, StoreError>;

    /// Retrieves an issue by its IssueId.
    async fn get_issue(&self, id: &IssueId) -> Result<Versioned<Issue>, StoreError>;

    /// Retrieves all versions of an issue in ascending version order.
    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError>;

    /// Updates an existing issue in the store.
    ///
    /// Returns an error if the issue does not exist or if any dependencies
    /// reference missing issues.
    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<(), StoreError>;

    /// Lists issues in the store that match the provided search query.
    ///
    /// By default, deleted issues are filtered out unless `include_deleted: true`
    /// is set in the query.
    ///
    /// Note: Graph filters (search_issue_graph) are handled separately as they
    /// require graph traversal that doesn't fit in the store layer.
    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError>;

    /// Soft-deletes an issue by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the issue with `deleted: true`.
    /// The issue can still be retrieved via `get_issue` but will be filtered
    /// from `list_issues` by default.
    async fn delete_issue(&self, id: &IssueId) -> Result<(), StoreError>;

    /// Applies dependency graph filters and returns the matching issue IDs.
    ///
    /// Filters are intersected, and any filter referencing a missing issue
    /// should return `StoreError::IssueNotFound`.
    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError>;

    /// Adds a new patch to the store and assigns it a PatchId.
    async fn add_patch(&self, patch: Patch) -> Result<PatchId, StoreError>;

    /// Retrieves a patch by its PatchId.
    async fn get_patch(&self, id: &PatchId) -> Result<Versioned<Patch>, StoreError>;

    /// Retrieves all versions of a patch in ascending version order.
    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError>;

    /// Updates an existing patch in the store.
    async fn update_patch(&self, id: &PatchId, patch: Patch) -> Result<(), StoreError>;

    /// Lists patches that match the provided search query.
    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError>;

    /// Soft-deletes a patch by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the patch with `deleted: true`.
    /// The patch can still be retrieved via `get_patch` but will be filtered
    /// from `list_patches` by default.
    async fn delete_patch(&self, id: &PatchId) -> Result<(), StoreError>;

    /// Lists all issues that reference the provided patch ID.
    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError>;

    /// Adds a new document to the store and assigns it a DocumentId.
    async fn add_document(&self, document: Document) -> Result<DocumentId, StoreError>;

    /// Retrieves a document by its DocumentId.
    ///
    /// # Arguments
    /// * `id` - The DocumentId to look up
    /// * `include_deleted` - If true, returns the document even if it has been soft-deleted.
    ///   If false, returns `StoreError::DocumentNotFound` for deleted documents.
    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError>;

    /// Retrieves all versions of a document in ascending order.
    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError>;

    /// Updates an existing document in the store.
    async fn update_document(&self, id: &DocumentId, document: Document) -> Result<(), StoreError>;

    /// Soft-deletes a document by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the document with `deleted: true`.
    /// The document can still be retrieved via `get_document` with `include_deleted: true`,
    /// but will be filtered from `get_document` with `include_deleted: false` and from
    /// `list_documents` by default (unless `include_deleted: true` is in the query).
    async fn delete_document(&self, id: &DocumentId) -> Result<(), StoreError>;

    /// Lists documents that match the provided search query.
    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError>;

    /// Returns documents that start with the provided path prefix.
    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError>;

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
    /// Tasks start in the Created state.
    /// # Arguments
    /// * `task` - The task to add
    /// * `creation_time` - The timestamp when the task is being created
    async fn add_task(
        &self,
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
        &self,
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
    /// The stored task version if successful, or an error if the task doesn't exist
    #[allow(dead_code)]
    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
    ) -> Result<Versioned<Task>, StoreError>;

    /// Gets a task by its TaskId.
    ///
    /// # Arguments
    /// * `id` - The TaskId to look up
    ///
    /// # Returns
    /// The task if found, or an error if not found
    async fn get_task(&self, id: &TaskId) -> Result<Versioned<Task>, StoreError>;

    /// Retrieves all versions of a task in ascending version order.
    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError>;

    /// Lists all tasks in the store that match the provided search query.
    ///
    /// # Arguments
    /// * `query` - Search query containing optional filters:
    ///   - `q`: Text search term that matches task ID, prompt, or status (case-insensitive)
    ///   - `spawned_from`: Filter tasks spawned from a specific issue
    ///   - `include_deleted`: Whether to include deleted tasks (default: false)
    ///
    /// Note: Text search does NOT match against notes since notes are derived
    /// from the status_log and not stored in the Task struct itself.
    ///
    /// # Returns
    /// A vector of all matching tasks in the store
    async fn list_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError>;

    /// Soft-deletes a task by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the task with `deleted: true`.
    /// The task can still be retrieved via `get_task` but will be filtered
    /// from `list_tasks` by default.
    async fn delete_task(&self, id: &TaskId) -> Result<(), StoreError>;

    /// Lists all task IDs with the specified status in the store.
    ///
    /// # Arguments
    /// * `status` - The status to filter by
    ///
    /// # Returns
    /// A vector of TaskIds for tasks with the specified status
    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError>;

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

    /// Adds a new actor to the store.
    async fn add_actor(&self, actor: Actor) -> Result<(), StoreError>;

    /// Updates an existing actor in the store.
    async fn update_actor(&self, actor: Actor) -> Result<(), StoreError>;

    /// Gets an actor by its canonical name.
    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError>;

    /// Lists all actors with their canonical names.
    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError>;

    /// Adds a new user to the store.
    ///
    /// If a user with the same username exists but is deleted, this will
    /// undelete the user by creating a new version with `deleted: false`.
    async fn add_user(&self, user: User) -> Result<(), StoreError>;

    /// Updates an existing user in the store.
    async fn update_user(&self, user: User) -> Result<Versioned<User>, StoreError>;

    /// Gets a user by their username.
    async fn get_user(&self, username: &Username) -> Result<Versioned<User>, StoreError>;

    /// Lists users that match the provided search query.
    ///
    /// By default, deleted users are filtered out unless `include_deleted: true`
    /// is set in the query.
    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError>;

    /// Soft-deletes a user by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the user with `deleted: true`.
    /// The user can still be retrieved via `get_user` but will be filtered
    /// from `list_users` by default.
    async fn delete_user(&self, username: &Username) -> Result<(), StoreError>;
}

pub use memory_store::MemoryStore;
