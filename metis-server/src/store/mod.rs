use crate::domain::{
    actors::{Actor, ActorError, ActorId, ActorRef},
    agents::Agent,
    documents::Document,
    issues::{Issue, IssueGraphFilter},
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::Patch,
    secrets::SecretRef,
    task_status::Event,
    users::{User, Username},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::sessions::SearchSessionsQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, SessionId,
    VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::{HashMap, HashSet};
use std::{fmt, str::FromStr};

pub(crate) mod issue_graph;
mod memory_store;
#[cfg(feature = "postgres")]
pub use crate::ee::store::migration;
#[cfg(feature = "postgres")]
pub use crate::ee::store::postgres_v2;
pub mod sqlite_store;

pub use crate::domain::sessions::Session;
pub use crate::domain::task_status::{Status, TaskError, TaskStatusLog};

/// The kind of object participating in a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Issue,
    Patch,
    Document,
}

impl ObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectKind::Issue => "issue",
            ObjectKind::Patch => "patch",
            ObjectKind::Document => "document",
        }
    }
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ObjectKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "issue" => Ok(ObjectKind::Issue),
            "patch" => Ok(ObjectKind::Patch),
            "document" => Ok(ObjectKind::Document),
            other => Err(format!("unsupported object kind '{other}'")),
        }
    }
}

/// Direction for transitive relationship traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitiveDirection {
    /// Follow source -> target edges (find all descendants).
    Forward,
    /// Follow target -> source edges (find all ancestors).
    Backward,
}

/// The type of relationship between two objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationshipType {
    ChildOf,
    BlockedOn,
    HasPatch,
    HasDocument,
}

impl RelationshipType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationshipType::ChildOf => "child-of",
            RelationshipType::BlockedOn => "blocked-on",
            RelationshipType::HasPatch => "has-patch",
            RelationshipType::HasDocument => "has-document",
        }
    }
}

impl fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RelationshipType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "child-of" | "childof" | "child_of" => Ok(RelationshipType::ChildOf),
            "blocked-on" | "blockedon" | "blocked_on" => Ok(RelationshipType::BlockedOn),
            "has-patch" | "haspatch" | "has_patch" => Ok(RelationshipType::HasPatch),
            "has-document" | "hasdocument" | "has_document" => Ok(RelationshipType::HasDocument),
            other => Err(format!("unsupported relationship type '{other}'")),
        }
    }
}

impl From<crate::domain::issues::IssueDependencyType> for RelationshipType {
    fn from(dep: crate::domain::issues::IssueDependencyType) -> Self {
        match dep {
            crate::domain::issues::IssueDependencyType::ChildOf => RelationshipType::ChildOf,
            crate::domain::issues::IssueDependencyType::BlockedOn => RelationshipType::BlockedOn,
        }
    }
}

/// A relationship between two objects in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRelationship {
    pub source_id: MetisId,
    pub source_kind: ObjectKind,
    pub target_id: MetisId,
    pub target_kind: ObjectKind,
    pub rel_type: RelationshipType,
}

pub(crate) fn validate_actor_name(name: &str) -> Result<(), StoreError> {
    match Actor::parse_name(name) {
        Ok(_) => Ok(()),
        Err(ActorError::InvalidActorName(name)) => Err(StoreError::InvalidActorName(name)),
    }
}

/// Maps a `Status` enum variant to the string used in the database.
pub(crate) fn status_to_db_str(status: Status) -> &'static str {
    match status {
        Status::Created => "created",
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Complete => "complete",
        Status::Failed => "failed",
    }
}

pub(crate) fn session_status_log_from_versions(
    versions: &[Versioned<Session>],
) -> Option<TaskStatusLog> {
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
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),
    #[error("Issue not found: {0}")]
    IssueNotFound(IssueId),
    #[error("Patch not found: {0}")]
    PatchNotFound(PatchId),
    #[error("Document not found: {0}")]
    DocumentNotFound(DocumentId),
    #[error("Message not found: {0}")]
    MessageNotFound(MessageId),
    #[error("Notification not found: {0}")]
    NotificationNotFound(NotificationId),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("Only one assignment agent is allowed")]
    AssignmentAgentAlreadyExists,
    #[error("Label not found: {0}")]
    LabelNotFound(LabelId),
    #[error("Label already exists: {0}")]
    LabelAlreadyExists(String),
    #[error("Invalid dependency: {0}")]
    InvalidDependency(IssueId),
    #[error("Invalid issue status: {0}")]
    InvalidIssueStatus(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Invalid status transition for session")]
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

/// Trait for read-only store operations: queries and lookups.
#[async_trait]
pub trait ReadOnlyStore: Send + Sync {
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

    /// Lists repository configurations keyed by name.
    ///
    /// By default, deleted repositories are filtered out unless `include_deleted: true`
    /// is set in the query.
    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError>;

    /// Retrieves an issue by its IssueId.
    ///
    /// # Arguments
    /// * `id` - The IssueId to look up
    /// * `include_deleted` - If true, returns the issue even if it has been soft-deleted.
    ///   If false, returns `StoreError::IssueNotFound` for deleted issues.
    async fn get_issue(
        &self,
        id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError>;

    /// Retrieves all versions of an issue in ascending version order.
    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError>;

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

    /// Counts issues matching the search query, ignoring pagination (cursor/limit).
    async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError>;

    /// Applies dependency graph filters and returns the matching issue IDs.
    ///
    /// Filters are intersected, and any filter referencing a missing issue
    /// should return `StoreError::IssueNotFound`.
    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError>;

    /// Lists all issues that declare the provided issue as a parent via `child-of`.
    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all issues that are blocked on the provided issue.
    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError>;

    /// Lists all session IDs spawned from the provided issue.
    async fn get_sessions_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<SessionId>, StoreError>;

    /// Retrieves a patch by its PatchId.
    ///
    /// # Arguments
    /// * `id` - The PatchId to look up
    /// * `include_deleted` - If true, returns the patch even if it has been soft-deleted.
    ///   If false, returns `StoreError::PatchNotFound` for deleted patches.
    async fn get_patch(
        &self,
        id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError>;

    /// Retrieves all versions of a patch in ascending version order.
    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError>;

    /// Lists patches that match the provided search query.
    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError>;

    /// Counts patches matching the search query, ignoring pagination (cursor/limit).
    async fn count_patches(&self, query: &SearchPatchesQuery) -> Result<u64, StoreError>;

    /// Lists all issues that reference the provided patch ID.
    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError>;

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

    /// Lists documents that match the provided search query.
    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError>;

    /// Counts documents matching the search query, ignoring pagination (cursor/limit).
    async fn count_documents(&self, query: &SearchDocumentsQuery) -> Result<u64, StoreError>;

    /// Returns documents that start with the provided path prefix.
    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError>;

    /// Gets a session by its SessionId.
    ///
    /// # Arguments
    /// * `id` - The SessionId to look up
    /// * `include_deleted` - If true, returns the session even if it has been soft-deleted.
    ///   If false, returns `StoreError::SessionNotFound` for deleted sessions.
    ///
    /// # Returns
    /// The session if found, or an error if not found
    async fn get_session(
        &self,
        id: &SessionId,
        include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError>;

    /// Retrieves all versions of a session in ascending version order.
    async fn get_session_versions(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError>;

    /// Lists all sessions in the store that match the provided search query.
    ///
    /// # Arguments
    /// * `query` - Search query containing optional filters:
    ///   - `q`: Text search term that matches session ID, prompt, or status (case-insensitive)
    ///   - `spawned_from`: Filter sessions spawned from a specific issue
    ///   - `include_deleted`: Whether to include deleted sessions (default: false)
    ///
    /// Note: Text search does NOT match against notes since notes are derived
    /// from the status_log and not stored in the Session struct itself.
    ///
    /// # Returns
    /// A vector of all matching sessions in the store
    async fn list_sessions(
        &self,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError>;

    /// Counts sessions matching the search query, ignoring pagination (cursor/limit).
    async fn count_sessions(&self, query: &SearchSessionsQuery) -> Result<u64, StoreError>;

    /// Gets the status log for a session by its SessionId.
    ///
    /// The status log contains timing information about the session's lifecycle,
    /// including when it was created, when it started running, when it completed,
    /// and any failure reason if applicable.
    ///
    /// # Arguments
    /// * `id` - The SessionId to look up
    ///
    /// # Returns
    /// The TaskStatusLog if found, or an error if not found
    async fn get_status_log(&self, id: &SessionId) -> Result<TaskStatusLog, StoreError>;

    /// Gets the status logs for multiple sessions in a single batch operation.
    ///
    /// Returns a map from SessionId to TaskStatusLog. Sessions that are not found
    /// are silently omitted from the result.
    async fn get_status_logs(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError>;

    /// Gets an actor by its canonical name.
    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError>;

    /// Lists all actors with their canonical names.
    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError>;

    /// Gets a user by their username.
    ///
    /// # Arguments
    /// * `username` - The Username to look up
    /// * `include_deleted` - If true, returns the user even if it has been soft-deleted.
    ///   If false, returns `StoreError::UserNotFound` for deleted users.
    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError>;

    /// Lists users that match the provided search query.
    ///
    /// By default, deleted users are filtered out unless `include_deleted: true`
    /// is set in the query.
    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError>;

    // ---- Notification (read-only) ----

    /// Retrieves a single notification by its ID.
    async fn get_notification(&self, id: &NotificationId) -> Result<Notification, StoreError>;

    /// Lists notifications matching the query, returning each notification with its ID.
    ///
    /// Results are ordered by `created_at DESC` (most recent first).
    async fn list_notifications(
        &self,
        query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError>;

    /// Counts unread notifications for the given recipient.
    async fn count_unread_notifications(&self, recipient: &ActorId) -> Result<u64, StoreError>;

    // ---- Message (read-only) ----

    /// Retrieves a message by its MessageId. Returns the latest version.
    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError>;

    /// Lists messages matching the search query, returning the latest version
    /// of each message in descending order (most recent first).
    async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError>;

    // ---- Agent (read-only) ----

    /// Retrieves an agent by its name.
    ///
    /// Returns `StoreError::AgentNotFound` if the agent does not exist or
    /// has been soft-deleted.
    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError>;

    /// Lists all non-deleted agents, ordered by name.
    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError>;

    // ---- Label (read-only) ----

    /// Retrieves a label by its LabelId.
    ///
    /// Returns `StoreError::LabelNotFound` if the label does not exist or
    /// has been soft-deleted.
    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError>;

    /// Lists labels matching the search query.
    ///
    /// By default, deleted labels are filtered out unless `include_deleted: true`
    /// is set in the query.
    async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError>;

    /// Counts labels matching the search query, ignoring pagination (cursor/limit).
    async fn count_labels(&self, query: &SearchLabelsQuery) -> Result<u64, StoreError>;

    /// Finds a label by its name (case-insensitive).
    ///
    /// Returns `None` if no non-deleted label with the given name exists.
    async fn get_label_by_name(&self, name: &str) -> Result<Option<(LabelId, Label)>, StoreError>;

    // ---- Label association (read-only) ----

    /// Returns all labels associated with the given object.
    async fn get_labels_for_object(
        &self,
        object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError>;

    /// Returns labels for multiple objects in a single batch query.
    ///
    /// Returns a map from object ID to its associated labels. Objects with
    /// no labels are omitted from the result.
    async fn get_labels_for_objects(
        &self,
        object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError>;

    /// Returns all object IDs associated with the given label.
    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<MetisId>, StoreError>;

    // ---- Object relationships (read-only) ----

    /// Returns object relationships matching the given filters.
    ///
    /// All provided filters are ANDed together. Pass `None` for any parameter
    /// to skip that filter.
    async fn get_relationships(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: Option<RelationshipType>,
    ) -> Result<Vec<ObjectRelationship>, StoreError>;

    /// Returns object relationships matching multiple source and/or target IDs.
    ///
    /// All provided filters are ANDed together. Pass `None` for any parameter
    /// to skip that filter.
    async fn get_relationships_batch(
        &self,
        source_ids: Option<&[MetisId]>,
        target_ids: Option<&[MetisId]>,
        rel_type: Option<RelationshipType>,
    ) -> Result<Vec<ObjectRelationship>, StoreError>;

    /// Returns object relationships reachable by transitively following edges
    /// of the given relationship type.
    ///
    /// Starting from `ids`, follows edges in `direction`:
    /// - `Forward`: follows source -> target edges (finds descendants)
    /// - `Backward`: follows target -> source edges (finds ancestors)
    async fn get_relationships_transitive(
        &self,
        ids: &[MetisId],
        direction: TransitiveDirection,
        rel_type: RelationshipType,
    ) -> Result<Vec<ObjectRelationship>, StoreError>;

    // ---- User secrets (read-only) ----

    /// Returns the encrypted value of a user secret, or None if not set.
    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError>;

    /// Lists the secrets configured for a user (not the values).
    async fn list_user_secret_names(
        &self,
        username: &Username,
    ) -> Result<Vec<SecretRef>, StoreError>;

    /// Checks whether a specific secret is marked as internal.
    ///
    /// Returns `Ok(true)` if the secret exists and is internal,
    /// `Ok(false)` if the secret does not exist or is not internal.
    async fn is_secret_internal(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<bool, StoreError>;
}

/// Trait for storing issues, patches, and sessions along with their statuses.
///
/// Implementations focus on persistence and referential integrity; application-specific
/// state transition rules (such as issue lifecycle validation) must be enforced by the
/// caller before invoking store operations.
#[async_trait]
pub trait Store: ReadOnlyStore {
    /// Adds a repository configuration under the provided name.
    ///
    /// Returns an error if a repository with the same name already exists.
    async fn add_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError>;

    /// Updates an existing repository configuration.
    ///
    /// Returns an error if the repository does not exist.
    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError>;

    /// Soft-deletes a repository by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the repository with `deleted: true`.
    /// The repository can still be retrieved via `get_repository` but will be filtered
    /// from `list_repositories` by default.
    async fn delete_repository(&self, name: &RepoName, actor: &ActorRef) -> Result<(), StoreError>;

    /// Adds a new issue to the store and assigns it an IssueId.
    ///
    /// Returns the new IssueId and its initial version number, or an error if
    /// any declared dependencies reference missing issues.
    async fn add_issue(
        &self,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError>;

    /// Updates an existing issue in the store.
    ///
    /// Returns the new version number, or an error if the issue does not exist
    /// or if any dependencies reference missing issues.
    async fn update_issue(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Soft-deletes an issue by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the issue with `deleted: true`.
    /// The issue can still be retrieved via `get_issue` but will be filtered
    /// from `list_issues` by default. Returns the version number of the
    /// deletion record.
    async fn delete_issue(
        &self,
        id: &IssueId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Adds a new patch to the store and assigns it a PatchId.
    ///
    /// Returns the new PatchId and its initial version number.
    async fn add_patch(
        &self,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError>;

    /// Updates an existing patch in the store.
    ///
    /// Returns the new version number.
    async fn update_patch(
        &self,
        id: &PatchId,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Soft-deletes a patch by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the patch with `deleted: true`.
    /// The patch can still be retrieved via `get_patch` but will be filtered
    /// from `list_patches` by default. Returns the version number of the
    /// deletion record.
    async fn delete_patch(
        &self,
        id: &PatchId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Adds a new document to the store and assigns it a DocumentId.
    ///
    /// Returns the new DocumentId and its initial version number.
    async fn add_document(
        &self,
        document: Document,
        actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError>;

    /// Updates an existing document in the store.
    ///
    /// Returns the new version number.
    async fn update_document(
        &self,
        id: &DocumentId,
        document: Document,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Soft-deletes a document by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the document with `deleted: true`.
    /// The document can still be retrieved via `get_document` with `include_deleted: true`,
    /// but will be filtered from `get_document` with `include_deleted: false` and from
    /// `list_documents` by default (unless `include_deleted: true` is in the query).
    /// Returns the version number of the deletion record.
    async fn delete_document(
        &self,
        id: &DocumentId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Adds a session to the store.
    ///
    /// Sessions start in the Created state.
    /// # Arguments
    /// * `session` - The session to add
    /// * `creation_time` - The timestamp when the session is being created
    /// * `actor` - The actor performing this mutation
    ///
    /// Returns the new SessionId and its initial version number.
    async fn add_session(
        &self,
        session: Session,
        creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(SessionId, VersionNumber), StoreError>;

    /// Updates an existing session in the store.
    ///
    /// This function overwrites the session data for the given vertex.
    ///
    /// # Arguments
    /// * `metis_id` - The SessionId of the session to update
    /// * `task` - The new Task to store for this vertex
    /// * `actor` - The actor performing this mutation
    ///
    /// # Returns
    /// The stored session version if successful, or an error if the session doesn't exist
    async fn update_session(
        &self,
        metis_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError>;

    /// Soft-deletes a session by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the session with `deleted: true`.
    /// The session can still be retrieved via `get_session` but will be filtered
    /// from `list_sessions` by default. Returns the version number of the
    /// deletion record.
    async fn delete_session(
        &self,
        id: &SessionId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Adds a new actor to the store.
    async fn add_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError>;

    /// Updates an existing actor in the store.
    async fn update_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError>;

    /// Adds a new user to the store.
    ///
    /// If a user with the same username exists but is deleted, this will
    /// undelete the user by creating a new version with `deleted: false`.
    async fn add_user(&self, user: User, actor: &ActorRef) -> Result<(), StoreError>;

    /// Updates an existing user in the store.
    async fn update_user(
        &self,
        user: User,
        actor: &ActorRef,
    ) -> Result<Versioned<User>, StoreError>;

    /// Soft-deletes a user by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the user with `deleted: true`.
    /// The user can still be retrieved via `get_user` with `include_deleted: true`,
    /// but will be filtered from `get_user` with `include_deleted: false` and from
    /// `list_users` by default (unless `include_deleted: true` is in the query).
    async fn delete_user(&self, username: &Username, actor: &ActorRef) -> Result<(), StoreError>;

    // ---- Notification mutations ----

    /// Inserts a new notification and returns the generated NotificationId.
    async fn insert_notification(
        &self,
        notification: Notification,
    ) -> Result<NotificationId, StoreError>;

    /// Marks a single notification as read.
    async fn mark_notification_read(&self, id: &NotificationId) -> Result<(), StoreError>;

    /// Marks all notifications as read for the given recipient.
    ///
    /// If `before` is provided, only notifications created before that timestamp are marked.
    /// Returns the number of notifications that were marked as read.
    async fn mark_all_notifications_read(
        &self,
        recipient: &ActorId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError>;

    // ---- Message mutations ----

    /// Adds a new message to the store at version 1.
    ///
    /// Returns the new MessageId and initial version number (1).
    async fn add_message(
        &self,
        message: Message,
        actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError>;

    /// Updates an existing message in the store, incrementing the version.
    ///
    /// Returns the new version number. Follows the same pattern as `update_issue`.
    async fn update_message(
        &self,
        id: &MessageId,
        message: Message,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    // ---- Agent mutations ----

    /// Adds a new agent to the store.
    ///
    /// Returns `StoreError::AgentAlreadyExists` if a non-deleted agent with
    /// the same name already exists.
    /// Returns `StoreError::AssignmentAgentAlreadyExists` if `is_assignment_agent`
    /// is true and another non-deleted agent already has this flag set.
    async fn add_agent(&self, agent: Agent) -> Result<(), StoreError>;

    /// Updates an existing agent.
    ///
    /// Returns `StoreError::AgentNotFound` if the agent does not exist.
    /// Returns `StoreError::AssignmentAgentAlreadyExists` if setting
    /// `is_assignment_agent` to true when another agent already has it set.
    async fn update_agent(&self, agent: Agent) -> Result<(), StoreError>;

    /// Soft-deletes an agent by setting its `deleted` flag to true.
    ///
    /// Returns `StoreError::AgentNotFound` if the agent does not exist.
    async fn delete_agent(&self, name: &str) -> Result<(), StoreError>;

    // ---- Label mutations ----

    /// Adds a new label to the store and assigns it a LabelId.
    ///
    /// Returns the new LabelId, or `StoreError::LabelAlreadyExists` if a
    /// non-deleted label with the same name already exists.
    async fn add_label(&self, label: Label) -> Result<LabelId, StoreError>;

    /// Updates an existing label's name and/or color.
    ///
    /// Returns `StoreError::LabelNotFound` if the label does not exist.
    /// Returns `StoreError::LabelAlreadyExists` if renaming to a name that
    /// is already taken by another non-deleted label.
    async fn update_label(&self, id: &LabelId, label: Label) -> Result<(), StoreError>;

    /// Soft-deletes a label by setting its `deleted` flag to true.
    async fn delete_label(&self, id: &LabelId) -> Result<(), StoreError>;

    // ---- Label association mutations ----

    /// Associates a label with an object. The object_kind is inferred from the
    /// MetisId prefix. Returns `true` if the association was newly created,
    /// `false` if it already existed.
    async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError>;

    /// Removes a label association. Returns `true` if the association was
    /// actually removed, `false` if it did not exist.
    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError>;

    // ---- Object relationship mutations ----

    /// Adds a relationship between two objects. Returns `true` if the
    /// relationship was newly created, `false` if it already existed.
    async fn add_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: RelationshipType,
    ) -> Result<bool, StoreError>;

    /// Removes a relationship between two objects. Returns `true` if the
    /// relationship was actually removed, `false` if it did not exist.
    async fn remove_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: RelationshipType,
    ) -> Result<bool, StoreError>;

    // ---- User secret mutations ----

    /// Sets (upserts) an encrypted secret for a user.
    async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
        internal: bool,
    ) -> Result<(), StoreError>;

    /// Deletes a user secret. No-ops if the secret does not exist.
    async fn delete_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<(), StoreError>;
}

/// Infers the object kind from a MetisId prefix.
///
/// Returns an error if the ID does not match a known object kind.
pub(crate) fn object_kind_from_id(id: &MetisId) -> Result<ObjectKind, StoreError> {
    let s: &str = id.as_ref();
    if s.starts_with(IssueId::prefix()) {
        Ok(ObjectKind::Issue)
    } else if s.starts_with(PatchId::prefix()) {
        Ok(ObjectKind::Patch)
    } else if s.starts_with(DocumentId::prefix()) {
        Ok(ObjectKind::Document)
    } else {
        Err(StoreError::Internal(format!(
            "unrecognized object id prefix: {s}"
        )))
    }
}

pub use memory_store::MemoryStore;
