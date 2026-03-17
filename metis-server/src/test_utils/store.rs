use crate::{
    domain::{
        actors::{Actor, ActorId, ActorRef},
        agents::Agent,
        documents::Document,
        issues::{Issue, IssueGraphFilter},
        labels::Label,
        messages::Message,
        notifications::Notification,
        patches::Patch,
        secrets::SecretRef,
        users::{User, Username},
    },
    store::{ReadOnlyStore, Session, Store, StoreError, TaskStatusLog},
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

    async fn search_issue_graph(
        &self,
        _filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
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

    async fn get_documents_by_path(
        &self,
        _path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
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

    async fn get_actor(&self, _name: &str) -> Result<Versioned<Actor>, StoreError> {
        crate::store::validate_actor_name(_name)?;
        fail()
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
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

    async fn get_message(&self, _id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        fail()
    }

    async fn list_messages(
        &self,
        _query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        fail()
    }

    async fn get_notification(&self, _id: &NotificationId) -> Result<Notification, StoreError> {
        fail()
    }

    async fn list_notifications(
        &self,
        _query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError> {
        fail()
    }

    async fn count_unread_notifications(&self, _recipient: &ActorId) -> Result<u64, StoreError> {
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
        _object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        fail()
    }

    async fn get_labels_for_objects(
        &self,
        _object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        fail()
    }

    async fn get_objects_for_label(&self, _label_id: &LabelId) -> Result<Vec<MetisId>, StoreError> {
        fail()
    }

    async fn get_relationships(
        &self,
        _source_id: Option<&MetisId>,
        _target_id: Option<&MetisId>,
        _rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        fail()
    }

    async fn get_relationships_batch(
        &self,
        _source_ids: Option<&[MetisId]>,
        _target_ids: Option<&[MetisId]>,
        _rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        fail()
    }

    async fn get_relationships_transitive(
        &self,
        _ids: &[MetisId],
        _direction: crate::store::TransitiveDirection,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
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

    async fn is_secret_internal(
        &self,
        _username: &Username,
        _secret_name: &str,
    ) -> Result<bool, StoreError> {
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
        _metis_id: &SessionId,
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

    async fn add_actor(&self, _actor: Actor, _acting_as: &ActorRef) -> Result<(), StoreError> {
        fail()
    }

    async fn update_actor(&self, _actor: Actor, _acting_as: &ActorRef) -> Result<(), StoreError> {
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

    async fn insert_notification(
        &self,
        _notification: Notification,
    ) -> Result<NotificationId, StoreError> {
        fail()
    }

    async fn mark_notification_read(&self, _id: &NotificationId) -> Result<(), StoreError> {
        fail()
    }

    async fn mark_all_notifications_read(
        &self,
        _recipient: &ActorId,
        _before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError> {
        fail()
    }

    async fn add_message(
        &self,
        _message: Message,
        _actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        fail()
    }

    async fn update_message(
        &self,
        _id: &MessageId,
        _message: Message,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
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
        _object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn remove_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn add_relationship(
        &self,
        _source_id: &MetisId,
        _target_id: &MetisId,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
        fail()
    }

    async fn remove_relationship(
        &self,
        _source_id: &MetisId,
        _target_id: &MetisId,
        _rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
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
}
