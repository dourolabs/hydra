use crate::domain::{
    actors::Actor,
    documents::Document,
    issues::{Issue, IssueGraphFilter},
    patches::Patch,
    users::{User, Username},
};
use crate::store::{Status, Store, StoreError, Task, TaskStatusLog};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, PatchId, RepoName, TaskId, Versioned,
    issues::IssueId,
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

/// Events emitted when server-side entities are mutated.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum ServerEvent {
    IssueCreated {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    IssueUpdated {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    IssueDeleted {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    PatchCreated {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    PatchUpdated {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    PatchDeleted {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    JobCreated {
        seq: u64,
        task_id: TaskId,
        timestamp: DateTime<Utc>,
    },
    JobUpdated {
        seq: u64,
        task_id: TaskId,
        timestamp: DateTime<Utc>,
    },
    DocumentCreated {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
    DocumentUpdated {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
    DocumentDeleted {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
}

impl ServerEvent {
    /// Returns the monotonic sequence number for this event.
    pub fn seq(&self) -> u64 {
        match self {
            ServerEvent::IssueCreated { seq, .. }
            | ServerEvent::IssueUpdated { seq, .. }
            | ServerEvent::IssueDeleted { seq, .. }
            | ServerEvent::PatchCreated { seq, .. }
            | ServerEvent::PatchUpdated { seq, .. }
            | ServerEvent::PatchDeleted { seq, .. }
            | ServerEvent::JobCreated { seq, .. }
            | ServerEvent::JobUpdated { seq, .. }
            | ServerEvent::DocumentCreated { seq, .. }
            | ServerEvent::DocumentUpdated { seq, .. }
            | ServerEvent::DocumentDeleted { seq, .. } => *seq,
        }
    }
}

const DEFAULT_BUFFER_SIZE: usize = 1024;

/// Broadcast-based event bus for notifying subscribers of entity mutations.
pub struct EventBus {
    sender: broadcast::Sender<ServerEvent>,
    next_seq: AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_BUFFER_SIZE);
        Self {
            sender,
            next_seq: AtomicU64::new(1),
        }
    }

    /// Returns a new receiver that will get all future events.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }

    /// Returns the current sequence number (the next seq that will be assigned).
    pub fn current_seq(&self) -> u64 {
        self.next_seq.load(Ordering::Relaxed)
    }

    /// Allocates the next monotonic sequence number.
    fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends an event on the bus. If there are no active receivers the event is
    /// silently dropped (this is normal during startup or when no SSE clients
    /// are connected).
    fn send(&self, event: ServerEvent) {
        let _ = self.sender.send(event);
    }

    pub fn emit_issue_created(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueCreated {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_issue_updated(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueUpdated {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_issue_deleted(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueDeleted {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_created(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchCreated {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_updated(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchUpdated {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_deleted(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchDeleted {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_job_created(&self, task_id: TaskId) {
        self.send(ServerEvent::JobCreated {
            seq: self.next_seq(),
            task_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_job_updated(&self, task_id: TaskId) {
        self.send(ServerEvent::JobUpdated {
            seq: self.next_seq(),
            task_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_created(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentCreated {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_updated(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentUpdated {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_deleted(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentDeleted {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A [`Store`] wrapper that automatically emits [`ServerEvent`]s on every
/// successful mutation. Read-only operations are forwarded unchanged.
pub struct StoreWithEvents {
    inner: Arc<dyn Store>,
    event_bus: Arc<EventBus>,
}

impl StoreWithEvents {
    pub fn new(inner: Arc<dyn Store>, event_bus: Arc<EventBus>) -> Self {
        Self { inner, event_bus }
    }

    /// Returns a reference to the underlying event bus.
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}

#[async_trait]
impl Store for StoreWithEvents {
    // ---- Repository (no events) ----

    async fn add_repository(&self, name: RepoName, config: Repository) -> Result<(), StoreError> {
        self.inner.add_repository(name, config).await
    }

    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        self.inner.get_repository(name, include_deleted).await
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
    ) -> Result<(), StoreError> {
        self.inner.update_repository(name, config).await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        self.inner.list_repositories(query).await
    }

    async fn delete_repository(&self, name: &RepoName) -> Result<(), StoreError> {
        self.inner.delete_repository(name).await
    }

    // ---- Issue ----

    async fn add_issue(&self, issue: Issue) -> Result<(IssueId, u64), StoreError> {
        let (issue_id, version) = self.inner.add_issue(issue).await?;
        self.event_bus.emit_issue_created(issue_id.clone());
        Ok((issue_id, version))
    }

    async fn get_issue(
        &self,
        id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        self.inner.get_issue(id, include_deleted).await
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        self.inner.get_issue_versions(id).await
    }

    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<u64, StoreError> {
        let version = self.inner.update_issue(id, issue).await?;
        self.event_bus.emit_issue_updated(id.clone());
        Ok(version)
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        self.inner.list_issues(query).await
    }

    async fn delete_issue(&self, id: &IssueId) -> Result<u64, StoreError> {
        let version = self.inner.delete_issue(id).await?;
        self.event_bus.emit_issue_deleted(id.clone());
        Ok(version)
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        self.inner.search_issue_graph(filters).await
    }

    // ---- Patch ----

    async fn add_patch(&self, patch: Patch) -> Result<(PatchId, u64), StoreError> {
        let (patch_id, version) = self.inner.add_patch(patch).await?;
        self.event_bus.emit_patch_created(patch_id.clone());
        Ok((patch_id, version))
    }

    async fn get_patch(
        &self,
        id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        self.inner.get_patch(id, include_deleted).await
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        self.inner.get_patch_versions(id).await
    }

    async fn update_patch(&self, id: &PatchId, patch: Patch) -> Result<u64, StoreError> {
        let version = self.inner.update_patch(id, patch).await?;
        self.event_bus.emit_patch_updated(id.clone());
        Ok(version)
    }

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        self.inner.list_patches(query).await
    }

    async fn delete_patch(&self, id: &PatchId) -> Result<u64, StoreError> {
        let version = self.inner.delete_patch(id).await?;
        self.event_bus.emit_patch_deleted(id.clone());
        Ok(version)
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issues_for_patch(patch_id).await
    }

    // ---- Document ----

    async fn add_document(&self, document: Document) -> Result<(DocumentId, u64), StoreError> {
        let (document_id, version) = self.inner.add_document(document).await?;
        self.event_bus.emit_document_created(document_id.clone());
        Ok((document_id, version))
    }

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        self.inner.get_document(id, include_deleted).await
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        self.inner.get_document_versions(id).await
    }

    async fn update_document(
        &self,
        id: &DocumentId,
        document: Document,
    ) -> Result<u64, StoreError> {
        let version = self.inner.update_document(id, document).await?;
        self.event_bus.emit_document_updated(id.clone());
        Ok(version)
    }

    async fn delete_document(&self, id: &DocumentId) -> Result<u64, StoreError> {
        let version = self.inner.delete_document(id).await?;
        self.event_bus.emit_document_deleted(id.clone());
        Ok(version)
    }

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        self.inner.list_documents(query).await
    }

    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        self.inner.get_documents_by_path(path_prefix).await
    }

    // ---- Issue graph queries ----

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_children(issue_id).await
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_blocked_on(issue_id).await
    }

    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        self.inner.get_tasks_for_issue(issue_id).await
    }

    // ---- Task/Job ----

    async fn add_task(
        &self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(TaskId, u64), StoreError> {
        let (task_id, version) = self.inner.add_task(task, creation_time).await?;
        self.event_bus.emit_job_created(task_id.clone());
        Ok((task_id, version))
    }

    async fn add_task_with_id(
        &self,
        metis_id: TaskId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner
            .add_task_with_id(metis_id.clone(), task, creation_time)
            .await?;
        self.event_bus.emit_job_created(metis_id);
        Ok(())
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
    ) -> Result<Versioned<Task>, StoreError> {
        let result = self.inner.update_task(metis_id, task).await?;
        self.event_bus.emit_job_updated(metis_id.clone());
        Ok(result)
    }

    async fn get_task(
        &self,
        id: &TaskId,
        include_deleted: bool,
    ) -> Result<Versioned<Task>, StoreError> {
        self.inner.get_task(id, include_deleted).await
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        self.inner.get_task_versions(id).await
    }

    async fn list_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        self.inner.list_tasks(query).await
    }

    async fn delete_task(&self, id: &TaskId) -> Result<u64, StoreError> {
        self.inner.delete_task(id).await
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        self.inner.list_tasks_with_status(status).await
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.inner.get_status_log(id).await
    }

    async fn get_status_logs(
        &self,
        ids: &[TaskId],
    ) -> Result<HashMap<TaskId, TaskStatusLog>, StoreError> {
        self.inner.get_status_logs(ids).await
    }

    // ---- Actor (no events) ----

    async fn add_actor(&self, actor: Actor) -> Result<(), StoreError> {
        self.inner.add_actor(actor).await
    }

    async fn update_actor(&self, actor: Actor) -> Result<(), StoreError> {
        self.inner.update_actor(actor).await
    }

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        self.inner.get_actor(name).await
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        self.inner.list_actors().await
    }

    // ---- User (no events) ----

    async fn add_user(&self, user: User) -> Result<(), StoreError> {
        self.inner.add_user(user).await
    }

    async fn update_user(&self, user: User) -> Result<Versioned<User>, StoreError> {
        self.inner.update_user(user).await
    }

    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        self.inner.get_user(username, include_deleted).await
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        self.inner.list_users(query).await
    }

    async fn delete_user(&self, username: &Username) -> Result<(), StoreError> {
        self.inner.delete_user(username).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    #[test]
    fn seq_numbers_are_monotonically_increasing() {
        let bus = EventBus::new();
        let s1 = bus.next_seq();
        let s2 = bus.next_seq();
        let s3 = bus.next_seq();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[tokio::test]
    async fn subscribe_receives_emitted_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue_id = IssueId::new();
        bus.emit_issue_created(issue_id.clone());

        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.seq(), 1);
        match event {
            ServerEvent::IssueCreated {
                issue_id: id, seq, ..
            } => {
                assert_eq!(id, issue_id);
                assert_eq!(seq, 1);
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_arrive_in_order() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue_id = IssueId::new();
        bus.emit_issue_created(issue_id.clone());
        bus.emit_issue_updated(issue_id);

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        assert!(e1.seq() < e2.seq());
        assert!(matches!(e1, ServerEvent::IssueCreated { .. }));
        assert!(matches!(e2, ServerEvent::IssueUpdated { .. }));
    }

    #[tokio::test]
    async fn store_with_events_emits_on_add_issue() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let (issue_id, _) = store.add_issue(issue).await.unwrap();

        let event = rx.recv().await.expect("should receive IssueCreated");
        assert!(
            matches!(&event, ServerEvent::IssueCreated { issue_id: id, .. } if *id == issue_id)
        );
    }

    #[tokio::test]
    async fn store_with_events_emits_on_update_issue() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let (issue_id, _) = store.add_issue(issue.clone()).await.unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let mut updated = issue;
        updated.status = IssueStatus::InProgress;
        store.update_issue(&issue_id, updated).await.unwrap();

        let event = rx.recv().await.expect("should receive IssueUpdated");
        assert!(
            matches!(&event, ServerEvent::IssueUpdated { issue_id: id, .. } if *id == issue_id)
        );
    }

    #[tokio::test]
    async fn store_with_events_emits_on_delete_issue() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let (issue_id, _) = store.add_issue(issue).await.unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        store.delete_issue(&issue_id).await.unwrap();

        let event = rx.recv().await.expect("should receive IssueDeleted");
        assert!(
            matches!(&event, ServerEvent::IssueDeleted { issue_id: id, .. } if *id == issue_id)
        );
    }
}
