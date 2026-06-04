use crate::domain::{
    actors::{Actor, ActorError, ActorRef},
    agents::Agent,
    conversations::Conversation,
    documents::Document,
    issues::Issue,
    labels::Label,
    patches::Patch,
    secrets::SecretRef,
    task_status::Event,
    users::{User, Username},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::documents::SearchDocumentsQuery;
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::api::v1::projects::{Project, ProjectKey};
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::api::v1::users::SearchUsersQuery;
use hydra_common::principal::Principal;
use hydra_common::triggers::Trigger;
use hydra_common::{
    ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, ProjectId, RepoName, SessionId,
    TriggerId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::HashMap;
use std::{fmt, str::FromStr};

mod memory_store;
pub mod migrations;
#[cfg(feature = "postgres")]
pub use crate::ee::store::postgres_v2;
pub mod sqlite_store;

pub use crate::domain::sessions::{
    AgentConfig, Session, SessionEvent, SessionEventSummary, SessionMode,
};
pub use crate::domain::task_status::{Status, TaskError, TaskStatusLog};

/// The kind of object participating in a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Issue,
    Patch,
    Document,
    Conversation,
    Trigger,
}

impl ObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectKind::Issue => "issue",
            ObjectKind::Patch => "patch",
            ObjectKind::Document => "document",
            ObjectKind::Conversation => "conversation",
            ObjectKind::Trigger => "trigger",
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
            "conversation" => Ok(ObjectKind::Conversation),
            "trigger" => Ok(ObjectKind::Trigger),
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
    RefersTo,
    Created,
}

impl RelationshipType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationshipType::ChildOf => "child-of",
            RelationshipType::BlockedOn => "blocked-on",
            RelationshipType::HasPatch => "has-patch",
            RelationshipType::HasDocument => "has-document",
            RelationshipType::RefersTo => "refers-to",
            RelationshipType::Created => "created",
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
            "refers-to" | "refersto" | "refers_to" => Ok(RelationshipType::RefersTo),
            "created" => Ok(RelationshipType::Created),
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
    pub source_id: HydraId,
    pub source_kind: ObjectKind,
    pub target_id: HydraId,
    pub target_kind: ObjectKind,
    pub rel_type: RelationshipType,
    pub created_at: DateTime<Utc>,
}

impl ObjectRelationship {
    /// Convert this store relationship into the wire `RelationResponse`.
    pub fn to_response(&self) -> hydra_common::api::v1::relations::RelationResponse {
        hydra_common::api::v1::relations::RelationResponse {
            source_id: self.source_id.clone(),
            target_id: self.target_id.clone(),
            rel_type: self.rel_type.as_str().to_string(),
            created_at: self.created_at,
        }
    }
}

/// An `auth_tokens` row resolved by token hash.
///
/// `session_id` records the session that minted the token (if any).
/// `is_revoked` is flipped on by `sessions/kill` for every token from
/// the killed session, and `require_auth` rejects any matched row whose
/// `is_revoked` is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTokenRow {
    pub actor_name: String,
    pub session_id: Option<SessionId>,
    pub is_revoked: bool,
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

/// Build the JSON value persisted to `tasks_v2.mount_spec` for a session on
/// dual-write inserts. `mount_spec` is the canonical mount source post-refactor.
pub(crate) fn dual_write_mount_spec_json(
    session: &Session,
) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(&session.mount_spec).map_err(|e| {
        StoreError::Internal(format!(
            "failed to serialize mount_spec for dual-write: {e}"
        ))
    })
}

/// Build the JSON value persisted to `tasks_v2.agent_config` for a session on
/// dual-write inserts. Now reads directly from `session.agent_config`.
pub(crate) fn dual_write_agent_config_json(
    session: &Session,
) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(&session.agent_config).map_err(|e| {
        StoreError::Internal(format!(
            "failed to serialize agent_config for dual-write: {e}"
        ))
    })
}

/// Build the JSON value persisted to `tasks_v2.mode` for a session on
/// dual-write inserts. Reads directly from `session.mode`.
pub(crate) fn dual_write_mode_json(session: &Session) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(&session.mode)
        .map_err(|e| StoreError::Internal(format!("failed to serialize mode for dual-write: {e}")))
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
    #[error("Conversation not found: {0}")]
    ConversationNotFound(ConversationId),
    #[error("Trigger not found: {0}")]
    TriggerNotFound(TriggerId),
    #[error("Project not found: {0}")]
    ProjectNotFound(ProjectId),
    #[error("Project key already in use: {0}")]
    ProjectKeyExists(ProjectKey),
    #[error("Issue not found: {0}")]
    IssueNotFound(IssueId),
    #[error("Patch not found: {0}")]
    PatchNotFound(PatchId),
    #[error("Document not found: {0}")]
    DocumentNotFound(DocumentId),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("Only one assignment agent is allowed")]
    AssignmentAgentAlreadyExists,
    #[error("Only one default conversation agent is allowed")]
    ConversationAgentAlreadyExists,
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
    #[error("A document already exists at this path")]
    DocumentPathConflict,
    /// Returned by store impls for methods that are not yet implemented in
    /// that backend. Used to keep trait parity while individual stores land
    /// behind separate PRs.
    #[error("Unsupported store operation: {0}")]
    Unsupported(&'static str),
}

impl StoreError {
    /// Returns a user-facing message for auth-middleware failures.
    ///
    /// Distinguishes invalid-credential errors (mapped to "authorization
    /// invalid") from other store failures (mapped to "authorization
    /// unavailable") so callers don't leak internal error detail.
    pub fn auth_failure_message(&self) -> &'static str {
        match self {
            StoreError::ActorNotFound(_)
            | StoreError::InvalidActorName(_)
            | StoreError::InvalidAuthToken => "authorization invalid",
            _ => "authorization unavailable",
        }
    }
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
    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError>;

    /// Counts issues matching the search query, ignoring pagination (cursor/limit).
    async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError>;

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

    /// Finds a non-deleted document with the exact given path.
    /// Returns the document ID and its latest version, or None if no such document exists.
    async fn find_non_deleted_document_by_exact_path(
        &self,
        path: &str,
    ) -> Result<Option<DocumentId>, StoreError>;

    /// Returns documents that start with the provided path prefix.
    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError>;

    /// Returns the live (non-deleted) document at each of the provided exact paths.
    ///
    /// Looks up documents whose `path` matches one of `paths` exactly. The result
    /// includes only paths that resolve to a live document — paths that do not
    /// match any non-deleted document are omitted. Duplicate paths in the input
    /// produce at most one result per path.
    async fn get_documents_by_paths(
        &self,
        paths: &[String],
    ) -> Result<Vec<(String, DocumentId, String)>, StoreError>;

    /// Returns the unique next-level path segments under the given prefix,
    /// along with the count of (non-deleted) documents under each segment.
    async fn list_document_path_children(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String, u64, bool)>, StoreError>;

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

    /// Look up a Hydra user from a raw GitHub login (case-insensitive).
    ///
    /// The GitHub PR poller maps `pr_review.user.login` to
    /// `Principal::User { name }` when a Hydra account exists for
    /// that login, and falls back to
    /// `Principal::External { system: "github", username }` otherwise.
    /// The users table does not carry a separate `github_login`
    /// column today (only `github_user_id`), so we treat the Hydra
    /// username as the GitHub login and match case-insensitively.
    ///
    /// Default implementation: try a direct (case-sensitive)
    /// [`Self::get_user`] first as the fast path, then fall back to
    /// a case-insensitive scan of [`Self::list_users`]. Stores that
    /// want a single-shot case-insensitive index lookup can override
    /// this method.
    ///
    /// Returns `Ok(None)` when no matching, non-deleted user exists.
    async fn get_user_by_github_login(
        &self,
        login: &str,
    ) -> Result<Option<Versioned<User>>, StoreError> {
        // Fast path: exact-case username match.
        let exact = Username::from(login);
        match self.get_user(&exact, false).await {
            Ok(user) => return Ok(Some(user)),
            Err(StoreError::UserNotFound(_)) => {}
            Err(other) => return Err(other),
        }

        // Fall back to a case-insensitive scan. Deleted users are filtered
        // out by `list_users` default (include_deleted = false), which
        // matches the design's intent: the poller should not attribute a
        // review to a tombstoned account.
        let users = self.list_users(&SearchUsersQuery::default()).await?;
        let login_lower = login.to_ascii_lowercase();
        Ok(users
            .into_iter()
            .find(|(name, _)| name.as_str().eq_ignore_ascii_case(&login_lower))
            .map(|(_, versioned)| versioned))
    }

    /// Returns whether the given [`Principal`] exists in the store.
    ///
    /// - `Principal::User { name }` → checks the users table
    ///   (`include_deleted = false`).
    /// - `Principal::Agent { name }` → checks the agents table.
    /// - `Principal::External { .. }` → always returns `true`. External
    ///   principals are format-checked only — they live in an external
    ///   identity provider by definition, so no DB lookup is meaningful.
    ///
    /// Used by `upsert_issue` to reject `assignee: Some(unknown)` with
    /// `UnknownAssignee` before persisting the row.
    async fn principal_exists(&self, principal: &Principal) -> Result<bool, StoreError> {
        match principal {
            Principal::User { name } => {
                // `name` is an API `Username`; cross over to the domain
                // type for the store lookup.
                let domain_name: Username = name.clone().into();
                match self.get_user(&domain_name, false).await {
                    Ok(_) => Ok(true),
                    Err(StoreError::UserNotFound(_)) => Ok(false),
                    Err(e) => Err(e),
                }
            }
            Principal::Agent { name } => match self.get_agent(name.as_str()).await {
                Ok(_) => Ok(true),
                Err(StoreError::AgentNotFound(_)) => Ok(false),
                Err(e) => Err(e),
            },
            Principal::External { .. } => Ok(true),
        }
    }

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
        object_id: &HydraId,
    ) -> Result<Vec<LabelSummary>, StoreError>;

    /// Returns labels for multiple objects in a single batch query.
    ///
    /// Returns a map from object ID to its associated labels. Objects with
    /// no labels are omitted from the result.
    async fn get_labels_for_objects(
        &self,
        object_ids: &[HydraId],
    ) -> Result<HashMap<HydraId, Vec<LabelSummary>>, StoreError>;

    /// Returns all object IDs associated with the given label.
    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<HydraId>, StoreError>;

    // ---- Conversation (read-only) ----

    /// Retrieves a conversation by its ConversationId.
    ///
    /// # Arguments
    /// * `id` - The ConversationId to look up
    /// * `include_deleted` - If true, returns the conversation even if it has been soft-deleted.
    ///   If false, returns `StoreError::ConversationNotFound` for deleted conversations.
    async fn get_conversation(
        &self,
        id: &ConversationId,
        include_deleted: bool,
    ) -> Result<Versioned<Conversation>, StoreError>;

    /// Lists conversations matching the query, returning summaries sorted by updated_at DESC.
    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<(ConversationId, Versioned<Conversation>)>, StoreError>;

    /// Retrieves all snapshot versions of a conversation, in version-number
    /// order, by reading the per-version rows persisted by
    /// [`Self::update_conversation`].
    ///
    /// Each row in `conversations` / `conversations_v2` (composite PK
    /// `(id, version_number)`) is a full conversation snapshot at that
    /// version, so the read path is a plain SELECT with no event-fold step.
    /// Status transitions (`Active`/`Idle`/`Closed`) are observable as new
    /// version rows on the conversation itself.
    ///
    /// Soft-deleted conversations are still returned via this method (callers
    /// that need the deleted filter should use
    /// [`Self::get_conversation`] separately).
    async fn get_conversation_versions(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<Versioned<Conversation>>, StoreError>;

    /// Returns event summaries for multiple conversations in a single batch
    /// operation.
    ///
    /// `event_count` is the count of chat-text `SessionEvent`s
    /// (`UserMessage` / `AssistantMessage`) summed across every session
    /// linked to the conversation. `ToolUse` and lifecycle session events
    /// (`Suspending` / `Resumed` / `Closed`) never contribute — only events
    /// that the chat UI surfaces as "messages" are counted.
    ///
    /// `last_event_preview` is the prefixed preview of the latest chat-text
    /// `SessionEvent` across the conversation's linked sessions — latest
    /// session first, then latest chat-text event within that session.
    /// `None` when no chat-text session event exists for the conversation.
    ///
    /// A conversation is omitted from the result entirely when it has no
    /// chat-text events — i.e. `event_count == 0` and `last_event_preview
    /// == None`.
    async fn get_conversation_event_summaries(
        &self,
        ids: &[ConversationId],
    ) -> Result<HashMap<ConversationId, ConversationEventSummary>, StoreError>;

    // ---- Session event log (read-only) ----

    /// Retrieves the append-only session event log for a session.
    ///
    /// Events are returned in append order.
    async fn get_session_events(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<SessionEvent>>, StoreError>;

    /// Look up every session linked to a conversation, in session-creation
    /// order. Backs the conversation read path in the sessions-orthogonality
    /// redesign §3.4.1 — a single query, no chain-walking.
    async fn list_session_ids_by_conversation_id(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<SessionId>, StoreError>;

    /// Returns session-event summaries (count + last event preview) for the
    /// provided session ids in a single batch. Sessions with no events are
    /// omitted from the result.
    async fn get_session_event_summaries(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, SessionEventSummary>, StoreError>;

    /// Retrieves the stored session-state blob for a session, if any.
    async fn get_session_state(&self, id: &SessionId) -> Result<Option<Vec<u8>>, StoreError>;

    // ---- Trigger (read-only) ----

    /// Retrieves a trigger by its TriggerId.
    ///
    /// # Arguments
    /// * `id` - The TriggerId to look up
    /// * `include_deleted` - If true, returns the trigger even if it has been soft-deleted.
    ///   If false, returns `StoreError::TriggerNotFound` for deleted triggers.
    async fn get_trigger(
        &self,
        id: &TriggerId,
        include_deleted: bool,
    ) -> Result<Versioned<Trigger>, StoreError>;

    /// Lists all triggers paired with their `TriggerId`.
    ///
    /// By default, deleted triggers are filtered out unless `include_deleted: true`.
    async fn list_triggers(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(TriggerId, Versioned<Trigger>)>, StoreError>;

    /// Retrieves all versions of a trigger in ascending version order.
    async fn get_trigger_versions(
        &self,
        id: &TriggerId,
    ) -> Result<Vec<Versioned<Trigger>>, StoreError>;

    // ---- Project (read-only) ----

    /// Retrieves a project by its [`ProjectId`].
    ///
    /// # Arguments
    /// * `id` - The ProjectId to look up
    /// * `include_deleted` - If true, returns the project even if it has been
    ///   soft-deleted. If false, returns `StoreError::ProjectNotFound` for
    ///   deleted projects.
    async fn get_project(
        &self,
        id: &ProjectId,
        include_deleted: bool,
    ) -> Result<Versioned<Project>, StoreError>;

    /// Lists all projects.
    ///
    /// By default, deleted projects are filtered out unless `include_deleted: true`.
    async fn list_projects(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(ProjectId, Versioned<Project>)>, StoreError>;

    // ---- Object relationships (read-only) ----

    /// Returns object relationships matching the given filters.
    ///
    /// All provided filters are ANDed together. Pass `None` for any parameter
    /// to skip that filter.
    async fn get_relationships(
        &self,
        source_id: Option<&HydraId>,
        target_id: Option<&HydraId>,
        rel_type: Option<RelationshipType>,
    ) -> Result<Vec<ObjectRelationship>, StoreError>;

    /// Returns object relationships matching multiple source and/or target IDs.
    ///
    /// All provided filters are ANDed together. Pass `None` for any parameter
    /// to skip that filter.
    async fn get_relationships_batch(
        &self,
        source_ids: Option<&[HydraId]>,
        target_ids: Option<&[HydraId]>,
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
        ids: &[HydraId],
        direction: TransitiveDirection,
        rel_type: RelationshipType,
    ) -> Result<Vec<ObjectRelationship>, StoreError>;

    // ---- Auth tokens (read-only) ----

    /// Returns all token hashes for the given actor.
    async fn get_auth_token_hashes(&self, actor_name: &str) -> Result<Vec<String>, StoreError>;

    /// Looks up an auth-token row by its hashed token.
    ///
    /// `require_auth` uses this to recover the session id that minted
    /// the token (when any) so the resulting `ActorRef::Authenticated`
    /// carries it into mutations.
    async fn get_auth_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<AuthTokenRow>, StoreError>;

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

    // ---- Conversation mutations ----

    /// Creates a new conversation in the store.
    ///
    /// The store generates and assigns the conversation ID.
    async fn add_conversation(
        &self,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<(ConversationId, VersionNumber), StoreError>;

    /// Updates an existing conversation. Takes the full conversation object.
    async fn update_conversation(
        &self,
        id: &ConversationId,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    // ---- Session event log mutations ----

    /// Appends an event to a session's event log. Returns the per-session
    /// version number assigned to the event.
    async fn append_session_event(
        &self,
        id: &SessionId,
        event: SessionEvent,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Stores the (binary, opaque) session-state blob for a session,
    /// overwriting any prior value.
    async fn store_session_state(
        &self,
        id: &SessionId,
        data: Vec<u8>,
        actor: &ActorRef,
    ) -> Result<(), StoreError>;

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
    /// * `hydra_id` - The SessionId of the session to update
    /// * `task` - The new Task to store for this vertex
    /// * `actor` - The actor performing this mutation
    ///
    /// # Returns
    /// The stored session version if successful, or an error if the session doesn't exist
    async fn update_session(
        &self,
        hydra_id: &SessionId,
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

    // ---- Agent mutations ----

    /// Adds a new agent to the store.
    ///
    /// Returns `StoreError::AgentAlreadyExists` if a non-deleted agent with
    /// the same name already exists.
    /// Returns `StoreError::AssignmentAgentAlreadyExists` if `is_assignment_agent`
    /// is true and another non-deleted agent already has this flag set.
    /// Returns `StoreError::ConversationAgentAlreadyExists` if
    /// `is_default_conversation_agent` is true and another non-deleted agent already
    /// has this flag set.
    async fn add_agent(&self, agent: Agent) -> Result<(), StoreError>;

    /// Updates an existing agent.
    ///
    /// Returns `StoreError::AgentNotFound` if the agent does not exist.
    /// Returns `StoreError::AssignmentAgentAlreadyExists` if setting
    /// `is_assignment_agent` to true when another agent already has it set.
    /// Returns `StoreError::ConversationAgentAlreadyExists` if setting
    /// `is_default_conversation_agent` to true when another agent already has it set.
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
    /// HydraId prefix. Returns `true` if the association was newly created,
    /// `false` if it already existed.
    async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<bool, StoreError>;

    /// Removes a label association. Returns `true` if the association was
    /// actually removed, `false` if it did not exist.
    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<bool, StoreError>;

    // ---- Trigger mutations ----

    /// Adds a new trigger to the store and assigns it a TriggerId.
    ///
    /// Returns the new TriggerId and its initial version number.
    async fn add_trigger(
        &self,
        trigger: Trigger,
        actor: &ActorRef,
    ) -> Result<(TriggerId, VersionNumber), StoreError>;

    /// Updates an existing trigger in the store.
    ///
    /// Returns the new version number, or `StoreError::TriggerNotFound` if
    /// the trigger does not exist.
    ///
    /// Any `last_fired_at` value supplied on `trigger` is **ignored**: the
    /// field is always taken from the current latest row inside the same
    /// transaction so a concurrent `record_trigger_fire` write is never
    /// regressed by a caller round-tripping a stale snapshot.
    async fn update_trigger(
        &self,
        id: &TriggerId,
        trigger: Trigger,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Soft-deletes a trigger by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the trigger with `deleted: true`.
    /// The trigger can still be retrieved via `get_trigger` with
    /// `include_deleted: true` but is filtered from `list_triggers` by
    /// default. Returns the version number of the deletion record.
    async fn delete_trigger(
        &self,
        id: &TriggerId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    // ---- Project mutations ----

    /// Adds a new project to the store and assigns it a [`ProjectId`].
    ///
    /// Returns the new ProjectId and its initial version number, or
    /// `StoreError::ProjectKeyExists` if a non-deleted project with the
    /// same [`ProjectKey`] already exists.
    async fn add_project(
        &self,
        project: Project,
        actor: &ActorRef,
    ) -> Result<(ProjectId, VersionNumber), StoreError>;

    /// Updates an existing project in the store.
    ///
    /// Returns the new version number, `StoreError::ProjectNotFound` if
    /// the project does not exist, or `StoreError::ProjectKeyExists` if
    /// the new [`ProjectKey`] would collide with another non-deleted
    /// project.
    async fn update_project(
        &self,
        id: &ProjectId,
        project: Project,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Soft-deletes a project by setting its `deleted` flag to true.
    ///
    /// This creates a new version of the project with `deleted: true`.
    /// The project can still be retrieved via `get_project` with
    /// `include_deleted: true` but is filtered from `list_projects` by
    /// default. Returns the version number of the deletion record.
    async fn delete_project(
        &self,
        id: &ProjectId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError>;

    /// Records that a trigger fired at `fired_at`, **in place** on the
    /// current latest row.
    ///
    /// This is the one carve-out from the versioned-append pattern:
    /// worker-driven bookkeeping (`last_fired_at`) is written in place,
    /// so no new version row is inserted, no `is_latest` flag flips, no
    /// actor is recorded, and no `VersionNumber` is returned. The
    /// trigger's version log then reflects only user-driven config
    /// changes. Concurrent `update_trigger` calls carry the latest
    /// `last_fired_at` forward into the new version row so neither
    /// change is lost.
    async fn record_trigger_fire(
        &self,
        id: &TriggerId,
        fired_at: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    // ---- Object relationship mutations ----

    /// Adds a relationship between two objects. Returns `true` if the
    /// relationship was newly created, `false` if it already existed.
    async fn add_relationship(
        &self,
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: RelationshipType,
    ) -> Result<bool, StoreError>;

    /// Removes a relationship between two objects. Returns `true` if the
    /// relationship was actually removed, `false` if it did not exist.
    async fn remove_relationship(
        &self,
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: RelationshipType,
    ) -> Result<bool, StoreError>;

    // ---- Auth token mutations ----

    /// Adds a token hash for the given actor.
    ///
    /// `session_id` records the session that minted the token. It is
    /// `Some` for the session-spawn path in `create_actor_for_job` and
    /// `None` for user logins or any other non-session token issuance.
    async fn add_auth_token(
        &self,
        actor_name: &str,
        token_hash: &str,
        session_id: Option<&SessionId>,
    ) -> Result<(), StoreError>;

    /// Deletes all auth tokens for the given actor.
    async fn delete_auth_tokens_for_actor(&self, actor_name: &str) -> Result<(), StoreError>;

    /// Marks every auth token minted by `session_id` as revoked.
    ///
    /// Called from `sessions/kill` so the killed session's container
    /// fails at the auth layer (401). The update is idempotent — a
    /// second call against the same session is a no-op because every
    /// matching row is already `is_revoked = true`.
    async fn revoke_auth_tokens_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), StoreError>;

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

/// Infers the object kind from a HydraId prefix.
///
/// Returns an error if the ID does not match a known object kind.
pub(crate) fn object_kind_from_id(id: &HydraId) -> Result<ObjectKind, StoreError> {
    let s: &str = id.as_ref();
    if s.starts_with(IssueId::prefix()) {
        Ok(ObjectKind::Issue)
    } else if s.starts_with(PatchId::prefix()) {
        Ok(ObjectKind::Patch)
    } else if s.starts_with(DocumentId::prefix()) {
        Ok(ObjectKind::Document)
    } else if s.starts_with(ConversationId::prefix()) {
        Ok(ObjectKind::Conversation)
    } else if s.starts_with(TriggerId::prefix()) {
        Ok(ObjectKind::Trigger)
    } else {
        Err(StoreError::Internal(format!(
            "unrecognized object id prefix: {s}"
        )))
    }
}

/// Summary of conversation events for batch fetching.
#[derive(Debug, Clone)]
pub struct ConversationEventSummary {
    pub event_count: usize,
    pub last_event_preview: Option<String>,
}

pub use memory_store::MemoryStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_kind_from_id_recognizes_conversation_prefix() {
        let conv_id = ConversationId::new();
        let hid = HydraId::from(conv_id);
        assert_eq!(object_kind_from_id(&hid).unwrap(), ObjectKind::Conversation);
    }

    #[test]
    fn object_kind_from_id_recognizes_trigger_prefix() {
        let trigger_id = TriggerId::new();
        let hid = HydraId::from(trigger_id);
        assert_eq!(object_kind_from_id(&hid).unwrap(), ObjectKind::Trigger);
    }

    #[test]
    fn object_kind_trigger_display_and_from_str() {
        assert_eq!(ObjectKind::Trigger.to_string(), "trigger");
        assert_eq!(
            "trigger".parse::<ObjectKind>().unwrap(),
            ObjectKind::Trigger
        );
    }

    #[test]
    fn relationship_type_created_display_and_from_str() {
        assert_eq!(RelationshipType::Created.to_string(), "created");
        assert_eq!(
            "created".parse::<RelationshipType>().unwrap(),
            RelationshipType::Created
        );
    }
}
