use crate::domain::conversations::Conversation;
use crate::{
    domain::{
        actors::ActorRef,
        agents::Agent,
        documents::Document,
        issues::Issue,
        labels::Label,
        patches::Patch,
        secrets::SecretRef,
        users::{User, Username},
    },
    store::{
        ConversationEventSummary, ReadOnlyStore, Session, SessionEvent, SessionEventSummary, Store,
        StoreError, TaskStatusLog,
    },
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::documents::SearchDocumentsQuery;
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::api::v1::projects::{Project, ProjectKey, StatusDefinition, StatusKey};
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::api::v1::users::SearchUsersQuery;
use hydra_common::triggers::Trigger;
use hydra_common::{
    ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, ProjectId, RepoName, SessionId,
    TriggerId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::HashMap;

/// Store implementation that always fails; useful for exercising error paths in tests.
#[derive(Default)]
pub struct FailingStore;

fn fail<T>() -> Result<T, StoreError> {
    Err(StoreError::Internal("forced failure".to_string()))
}

#[async_trait]
impl ReadOnlyStore for FailingStore {
    async fn get_repository(
        &self,
        _name: &RepoName,
        _include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        fail()
    }

    async fn list_repositories(
        &self,
        _query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        fail()
    }

    async fn get_issue(
        &self,
        _id: &IssueId,
        _include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        fail()
    }

    async fn get_issue_versions(&self, _id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        fail()
    }

    async fn list_issues(
        &self,
        _query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        fail()
    }

    async fn count_issues(&self, _query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        fail()
    }

    async fn list_stale_issues_for_status(
        &self,
        _project_id: &ProjectId,
        _status_key: &StatusKey,
        _threshold_seconds: i64,
        _now: DateTime<Utc>,
        _limit: u32,
    ) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn count_active_sessions_in_status(
        &self,
        _project_id: &ProjectId,
        _status_key: &StatusKey,
    ) -> Result<u64, StoreError> {
        fail()
    }

    async fn get_issue_children(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_issue_blocked_on(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_sessions_for_issue(
        &self,
        _issue_id: &IssueId,
    ) -> Result<Vec<SessionId>, StoreError> {
        fail()
    }

    async fn list_comments(
        &self,
        _issue_id: &IssueId,
        _limit: u32,
        _before_sequence: Option<u64>,
    ) -> Result<crate::domain::comments::ListCommentsPage, StoreError> {
        fail()
    }

    async fn get_patch(
        &self,
        _id: &PatchId,
        _include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        fail()
    }

    async fn get_patch_versions(&self, _id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        fail()
    }

    async fn list_patches(
        &self,
        _query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        fail()
    }

    async fn count_patches(&self, _query: &SearchPatchesQuery) -> Result<u64, StoreError> {
        fail()
    }

    async fn get_issues_for_patch(&self, _patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_document(
        &self,
        _id: &DocumentId,
        _include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        fail()
    }

    async fn get_document_versions(
        &self,
        _id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        fail()
    }

    async fn list_documents(
        &self,
        _query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        fail()
    }

    async fn count_documents(&self, _query: &SearchDocumentsQuery) -> Result<u64, StoreError> {
        fail()
    }

    async fn find_non_deleted_document_by_exact_path(
        &self,
        _path: &str,
    ) -> Result<Option<DocumentId>, StoreError> {
        fail()
    }

    async fn get_documents_by_path(
        &self,
        _path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        fail()
    }

    async fn get_documents_by_paths(
        &self,
        _paths: &[String],
    ) -> Result<Vec<(String, DocumentId, String)>, StoreError> {
        fail()
    }

    async fn list_document_path_children(
        &self,
        _prefix: &str,
    ) -> Result<Vec<(String, String, u64, bool)>, StoreError> {
        fail()
    }

    async fn get_session(
        &self,
        _id: &SessionId,
        _include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError> {
        fail()
    }

    async fn get_session_versions(
        &self,
        _id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError> {
        fail()
    }

    async fn list_sessions(
        &self,
        _query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError> {
        fail()
    }

    async fn count_sessions(&self, _query: &SearchSessionsQuery) -> Result<u64, StoreError> {
        fail()
    }

    async fn get_status_log(&self, _id: &SessionId) -> Result<TaskStatusLog, StoreError> {
        fail()
    }

    async fn get_status_logs(
        &self,
        _ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError> {
        fail()
    }

    async fn get_user(
        &self,
        _username: &Username,
        _include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        fail()
    }

    async fn list_users(
        &self,
        _query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        fail()
    }

    async fn get_agent(&self, _name: &str) -> Result<Agent, StoreError> {
        fail()
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        fail()
    }

    async fn get_label(&self, _id: &LabelId) -> Result<Label, StoreError> {
        fail()
    }

    async fn list_labels(
        &self,
        _query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        fail()
    }

    async fn count_labels(&self, _query: &SearchLabelsQuery) -> Result<u64, StoreError> {
        fail()
    }

    async fn get_label_by_name(&self, _name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        fail()
    }

    async fn get_labels_for_object(
        &self,
        _object_id: &HydraId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        fail()
    }

    async fn get_labels_for_objects(
        &self,
        _object_ids: &[HydraId],
    ) -> Result<HashMap<HydraId, Vec<LabelSummary>>, StoreError> {
        fail()
    }

    async fn get_objects_for_label(&self, _label_id: &LabelId) -> Result<Vec<HydraId>, StoreError> {
        fail()
    }

    async fn get_trigger(
        &self,
        _id: &TriggerId,
        _include_deleted: bool,
    ) -> Result<Versioned<Trigger>, StoreError> {
        fail()
    }

    async fn list_triggers(
        &self,
        _include_deleted: bool,
    ) -> Result<Vec<(TriggerId, Versioned<Trigger>)>, StoreError> {
        fail()
    }

    async fn get_trigger_versions(
        &self,
        _id: &TriggerId,
    ) -> Result<Vec<Versioned<Trigger>>, StoreError> {
        fail()
    }

    async fn get_project(
        &self,
        _id: &ProjectId,
        _include_archived: bool,
    ) -> Result<Versioned<Project>, StoreError> {
        fail()
    }

    async fn get_project_by_key(
        &self,
        _key: &ProjectKey,
        _include_archived: bool,
    ) -> Result<Option<(ProjectId, Versioned<Project>)>, StoreError> {
        fail()
    }

    async fn list_projects(
        &self,
        _include_archived: bool,
    ) -> Result<Vec<(ProjectId, Versioned<Project>)>, StoreError> {
        fail()
    }

    async fn get_relationships(
        &self,
        _source_id: Option<&HydraId>,
        _target_id: Option<&HydraId>,
        _rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        fail()
    }

    async fn get_relationships_batch(
        &self,
        _source_ids: Option<&[HydraId]>,
        _target_ids: Option<&[HydraId]>,
        _rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        fail()
    }

    async fn get_relationships_transitive(
        &self,
        _ids: &[HydraId],
        _direction: crate::store::TransitiveDirection,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        fail()
    }

    async fn get_auth_token_hashes(&self, _actor_name: &str) -> Result<Vec<String>, StoreError> {
        fail()
    }

    async fn get_auth_token_by_hash(
        &self,
        _token_hash: &str,
    ) -> Result<Option<crate::store::AuthTokenRow>, StoreError> {
        fail()
    }

    async fn get_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        fail()
    }

    async fn list_user_secret_names(
        &self,
        _username: &Username,
    ) -> Result<Vec<SecretRef>, StoreError> {
        fail()
    }

    async fn get_conversation(
        &self,
        _id: &ConversationId,
        _include_deleted: bool,
    ) -> Result<Versioned<Conversation>, StoreError> {
        fail()
    }

    async fn list_conversations(
        &self,
        _query: &SearchConversationsQuery,
    ) -> Result<Vec<(ConversationId, Versioned<Conversation>)>, StoreError> {
        fail()
    }

    async fn get_conversation_versions(
        &self,
        _id: &ConversationId,
    ) -> Result<Vec<Versioned<Conversation>>, StoreError> {
        fail()
    }

    async fn get_conversation_event_summaries(
        &self,
        _ids: &[ConversationId],
    ) -> Result<HashMap<ConversationId, ConversationEventSummary>, StoreError> {
        fail()
    }

    async fn get_session_events(
        &self,
        _id: &SessionId,
    ) -> Result<Vec<Versioned<SessionEvent>>, StoreError> {
        fail()
    }

    async fn list_session_ids_by_conversation_id(
        &self,
        _conversation_id: &ConversationId,
    ) -> Result<Vec<SessionId>, StoreError> {
        fail()
    }

    async fn get_session_event_summaries(
        &self,
        _ids: &[SessionId],
    ) -> Result<HashMap<SessionId, SessionEventSummary>, StoreError> {
        fail()
    }

    async fn get_session_state(&self, _id: &SessionId) -> Result<Option<Vec<u8>>, StoreError> {
        fail()
    }
}

#[async_trait]
impl Store for FailingStore {
    async fn add_repository(
        &self,
        _name: RepoName,
        _config: Repository,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn update_repository(
        &self,
        _name: RepoName,
        _config: Repository,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn delete_repository(
        &self,
        _name: &RepoName,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn add_issue(
        &self,
        _issue: Issue,
        _actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_issue(
        &self,
        _id: &IssueId,
        _issue: Issue,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn delete_issue(
        &self,
        _id: &IssueId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_patch(
        &self,
        _patch: Patch,
        _actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_patch(
        &self,
        _id: &PatchId,
        _patch: Patch,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn delete_patch(
        &self,
        _id: &PatchId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_document(
        &self,
        _document: Document,
        _actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_document(
        &self,
        _id: &DocumentId,
        _document: Document,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn delete_document(
        &self,
        _id: &DocumentId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_session(
        &self,
        _session: Session,
        _creation_time: DateTime<Utc>,
        _actor: &ActorRef,
    ) -> Result<(SessionId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_session(
        &self,
        _hydra_id: &SessionId,
        _session: Session,
        _actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        fail()
    }

    async fn delete_session(
        &self,
        _id: &SessionId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_user(&self, _user: User, _actor: &ActorRef) -> Result<(), StoreError> {
        fail()
    }

    async fn update_user(
        &self,
        _user: User,
        _actor: &ActorRef,
    ) -> Result<Versioned<User>, StoreError> {
        fail()
    }

    async fn delete_user(&self, _username: &Username, _actor: &ActorRef) -> Result<(), StoreError> {
        fail()
    }

    async fn add_agent(&self, _agent: Agent) -> Result<(), StoreError> {
        fail()
    }

    async fn update_agent(&self, _agent: Agent) -> Result<(), StoreError> {
        fail()
    }

    async fn delete_agent(&self, _name: &str) -> Result<(), StoreError> {
        fail()
    }

    async fn add_label(&self, _label: Label) -> Result<LabelId, StoreError> {
        fail()
    }

    async fn update_label(&self, _id: &LabelId, _label: Label) -> Result<(), StoreError> {
        fail()
    }

    async fn delete_label(&self, _id: &LabelId) -> Result<(), StoreError> {
        fail()
    }

    async fn add_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &HydraId,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn remove_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &HydraId,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn add_relationship(
        &self,
        _source_id: &HydraId,
        _target_id: &HydraId,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn remove_relationship(
        &self,
        _source_id: &HydraId,
        _target_id: &HydraId,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn add_auth_token(
        &self,
        _actor_name: &str,
        _token_hash: &str,
        _session_id: Option<&SessionId>,
        _creator: &Username,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn delete_auth_tokens_for_actor(&self, _actor_name: &str) -> Result<(), StoreError> {
        fail()
    }

    async fn revoke_auth_tokens_for_session(
        &self,
        _session_id: &SessionId,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn set_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
        _encrypted_value: &[u8],
        _internal: bool,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn delete_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn add_conversation(
        &self,
        _conversation: Conversation,
        _actor: &ActorRef,
    ) -> Result<(ConversationId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_conversation(
        &self,
        _id: &ConversationId,
        _conversation: Conversation,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn append_session_event(
        &self,
        _id: &SessionId,
        _event: SessionEvent,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn store_session_state(
        &self,
        _id: &SessionId,
        _data: Vec<u8>,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn add_trigger(
        &self,
        _trigger: Trigger,
        _actor: &ActorRef,
    ) -> Result<(TriggerId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_trigger(
        &self,
        _id: &TriggerId,
        _trigger: Trigger,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn delete_trigger(
        &self,
        _id: &TriggerId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn record_trigger_fire(
        &self,
        _id: &TriggerId,
        _fired_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn add_project(
        &self,
        _project: Project,
        _actor: &ActorRef,
    ) -> Result<(ProjectId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_project(
        &self,
        _id: &ProjectId,
        _project: Project,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn delete_project(
        &self,
        _id: &ProjectId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_status(
        &self,
        _id: &ProjectId,
        _status: StatusDefinition,
        _actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        fail()
    }

    async fn update_status(
        &self,
        _id: &ProjectId,
        _status_key: &StatusKey,
        _status: StatusDefinition,
        _actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        fail()
    }

    async fn delete_status(
        &self,
        _id: &ProjectId,
        _status_key: &StatusKey,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        fail()
    }

    async fn add_comment(
        &self,
        _issue_id: &IssueId,
        _body: String,
        _actor: &ActorRef,
    ) -> Result<crate::domain::comments::Comment, StoreError> {
        fail()
    }
}
