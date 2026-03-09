use crate::domain::{
    actors::{Actor, ActorId, ActorRef},
    agents::Agent,
    documents::Document,
    issues::{Issue, IssueGraphFilter},
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::Patch,
    users::{User, Username},
};
use crate::store::{ReadOnlyStore, RelationshipType, Store, StoreError, Task, TaskStatusLog};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, TaskId,
    VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    issues::IssueId,
    repositories::{Repository, SearchRepositoriesQuery},
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

/// Payload carrying before/after entity state for mutation events.
///
/// Wrapped in `Arc` inside `ServerEvent` so that cloning events to multiple
/// broadcast receivers is cheap regardless of payload size.
#[derive(Debug, Clone)]
pub enum MutationPayload {
    Issue {
        old: Option<Issue>,
        new: Issue,
        actor: ActorRef,
    },
    Patch {
        old: Option<Patch>,
        new: Patch,
        actor: ActorRef,
    },
    Job {
        old: Option<Task>,
        new: Task,
        actor: ActorRef,
    },
    Document {
        old: Option<Document>,
        new: Document,
        actor: ActorRef,
    },
    Message {
        old: Option<Message>,
        new: Message,
        actor: ActorRef,
    },
    Label {
        old: Option<Label>,
        new: Label,
        actor: ActorRef,
    },
    Notification {
        old: Option<Notification>,
        new: Notification,
        actor: ActorRef,
    },
}

impl MutationPayload {
    /// Returns the actor reference associated with this mutation.
    pub fn actor(&self) -> &ActorRef {
        match self {
            MutationPayload::Issue { actor, .. }
            | MutationPayload::Patch { actor, .. }
            | MutationPayload::Job { actor, .. }
            | MutationPayload::Document { actor, .. }
            | MutationPayload::Label { actor, .. }
            | MutationPayload::Message { actor, .. }
            | MutationPayload::Notification { actor, .. } => actor,
        }
    }
}

/// Data-free mirror of [`ServerEvent`] used for event filtering without
/// needing to construct dummy/sentinel instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventType {
    IssueCreated,
    IssueUpdated,
    IssueDeleted,
    PatchCreated,
    PatchUpdated,
    PatchDeleted,
    JobCreated,
    JobUpdated,
    DocumentCreated,
    DocumentUpdated,
    DocumentDeleted,
    LabelCreated,
    LabelUpdated,
    LabelDeleted,
    MessageCreated,
    MessageUpdated,
    NotificationCreated,
}

/// Events emitted when server-side entities are mutated.
///
/// Each variant carries an optional `payload` wrapped in `Arc<MutationPayload>`
/// containing the before/after entity state. For update events, `old` is the
/// state before the mutation; for create events, only `new` is set; for delete
/// events, `old` holds the entity as it was before deletion and `new` holds
/// the deleted (soft-deleted) version.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum ServerEvent {
    IssueCreated {
        seq: u64,
        issue_id: IssueId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    IssueUpdated {
        seq: u64,
        issue_id: IssueId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    IssueDeleted {
        seq: u64,
        issue_id: IssueId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    PatchCreated {
        seq: u64,
        patch_id: PatchId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    PatchUpdated {
        seq: u64,
        patch_id: PatchId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    PatchDeleted {
        seq: u64,
        patch_id: PatchId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    JobCreated {
        seq: u64,
        task_id: TaskId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    JobUpdated {
        seq: u64,
        task_id: TaskId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    DocumentCreated {
        seq: u64,
        document_id: DocumentId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    DocumentUpdated {
        seq: u64,
        document_id: DocumentId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    DocumentDeleted {
        seq: u64,
        document_id: DocumentId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    LabelCreated {
        seq: u64,
        label_id: LabelId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    LabelUpdated {
        seq: u64,
        label_id: LabelId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    LabelDeleted {
        seq: u64,
        label_id: LabelId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    MessageCreated {
        seq: u64,
        message_id: MessageId,
        recipient: ActorId,
        sender: Option<ActorId>,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    MessageUpdated {
        seq: u64,
        message_id: MessageId,
        recipient: ActorId,
        sender: Option<ActorId>,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
    },
    NotificationCreated {
        seq: u64,
        notification_id: NotificationId,
        version: u64,
        timestamp: DateTime<Utc>,
        payload: Arc<MutationPayload>,
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
            | ServerEvent::DocumentDeleted { seq, .. }
            | ServerEvent::LabelCreated { seq, .. }
            | ServerEvent::LabelUpdated { seq, .. }
            | ServerEvent::LabelDeleted { seq, .. }
            | ServerEvent::MessageCreated { seq, .. }
            | ServerEvent::MessageUpdated { seq, .. }
            | ServerEvent::NotificationCreated { seq, .. } => *seq,
        }
    }

    /// Returns a reference to the mutation payload for this event.
    pub fn payload(&self) -> &Arc<MutationPayload> {
        match self {
            ServerEvent::IssueCreated { payload, .. }
            | ServerEvent::IssueUpdated { payload, .. }
            | ServerEvent::IssueDeleted { payload, .. }
            | ServerEvent::PatchCreated { payload, .. }
            | ServerEvent::PatchUpdated { payload, .. }
            | ServerEvent::PatchDeleted { payload, .. }
            | ServerEvent::JobCreated { payload, .. }
            | ServerEvent::JobUpdated { payload, .. }
            | ServerEvent::DocumentCreated { payload, .. }
            | ServerEvent::DocumentUpdated { payload, .. }
            | ServerEvent::DocumentDeleted { payload, .. }
            | ServerEvent::LabelCreated { payload, .. }
            | ServerEvent::LabelUpdated { payload, .. }
            | ServerEvent::LabelDeleted { payload, .. }
            | ServerEvent::MessageCreated { payload, .. }
            | ServerEvent::MessageUpdated { payload, .. }
            | ServerEvent::NotificationCreated { payload, .. } => payload,
        }
    }

    /// Returns the data-free [`EventType`] corresponding to this event variant.
    pub fn event_type(&self) -> EventType {
        match self {
            ServerEvent::IssueCreated { .. } => EventType::IssueCreated,
            ServerEvent::IssueUpdated { .. } => EventType::IssueUpdated,
            ServerEvent::IssueDeleted { .. } => EventType::IssueDeleted,
            ServerEvent::PatchCreated { .. } => EventType::PatchCreated,
            ServerEvent::PatchUpdated { .. } => EventType::PatchUpdated,
            ServerEvent::PatchDeleted { .. } => EventType::PatchDeleted,
            ServerEvent::JobCreated { .. } => EventType::JobCreated,
            ServerEvent::JobUpdated { .. } => EventType::JobUpdated,
            ServerEvent::DocumentCreated { .. } => EventType::DocumentCreated,
            ServerEvent::DocumentUpdated { .. } => EventType::DocumentUpdated,
            ServerEvent::DocumentDeleted { .. } => EventType::DocumentDeleted,
            ServerEvent::LabelCreated { .. } => EventType::LabelCreated,
            ServerEvent::LabelUpdated { .. } => EventType::LabelUpdated,
            ServerEvent::LabelDeleted { .. } => EventType::LabelDeleted,
            ServerEvent::MessageCreated { .. } => EventType::MessageCreated,
            ServerEvent::MessageUpdated { .. } => EventType::MessageUpdated,
            ServerEvent::NotificationCreated { .. } => EventType::NotificationCreated,
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

    pub fn emit_issue_created(
        &self,
        issue_id: IssueId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::IssueCreated {
            seq: self.next_seq(),
            issue_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_issue_updated(
        &self,
        issue_id: IssueId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::IssueUpdated {
            seq: self.next_seq(),
            issue_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_issue_deleted(
        &self,
        issue_id: IssueId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::IssueDeleted {
            seq: self.next_seq(),
            issue_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_patch_created(
        &self,
        patch_id: PatchId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::PatchCreated {
            seq: self.next_seq(),
            patch_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_patch_updated(
        &self,
        patch_id: PatchId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::PatchUpdated {
            seq: self.next_seq(),
            patch_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_patch_deleted(
        &self,
        patch_id: PatchId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::PatchDeleted {
            seq: self.next_seq(),
            patch_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_job_created(&self, task_id: TaskId, version: u64, payload: Arc<MutationPayload>) {
        self.send(ServerEvent::JobCreated {
            seq: self.next_seq(),
            task_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_job_updated(&self, task_id: TaskId, version: u64, payload: Arc<MutationPayload>) {
        self.send(ServerEvent::JobUpdated {
            seq: self.next_seq(),
            task_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_document_created(
        &self,
        document_id: DocumentId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::DocumentCreated {
            seq: self.next_seq(),
            document_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_document_updated(
        &self,
        document_id: DocumentId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::DocumentUpdated {
            seq: self.next_seq(),
            document_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_document_deleted(
        &self,
        document_id: DocumentId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::DocumentDeleted {
            seq: self.next_seq(),
            document_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_label_created(
        &self,
        label_id: LabelId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::LabelCreated {
            seq: self.next_seq(),
            label_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_label_updated(
        &self,
        label_id: LabelId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::LabelUpdated {
            seq: self.next_seq(),
            label_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_label_deleted(
        &self,
        label_id: LabelId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::LabelDeleted {
            seq: self.next_seq(),
            label_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_message_created(
        &self,
        message_id: MessageId,
        recipient: ActorId,
        sender: Option<ActorId>,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::MessageCreated {
            seq: self.next_seq(),
            message_id,
            recipient,
            sender,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_message_updated(
        &self,
        message_id: MessageId,
        recipient: ActorId,
        sender: Option<ActorId>,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::MessageUpdated {
            seq: self.next_seq(),
            message_id,
            recipient,
            sender,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }

    pub fn emit_notification_created(
        &self,
        notification_id: NotificationId,
        version: u64,
        payload: Arc<MutationPayload>,
    ) {
        self.send(ServerEvent::NotificationCreated {
            seq: self.next_seq(),
            notification_id,
            version,
            timestamp: Utc::now(),
            payload,
        });
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A wrapper around a [`Store`] that automatically emits [`ServerEvent`]s on
/// every successful mutation. All mutations require an explicit `actor`
/// parameter ([`ActorRef`]) so that event payloads always carry actor
/// provenance. Read-only operations are forwarded unchanged via the
/// [`ReadOnlyStore`] trait implementation.
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

    // ---- Actor-aware mutation methods ----
    //
    // These inherent methods accept an explicit `actor: ActorRef` parameter
    // that is included in the emitted `MutationPayload`.

    pub async fn add_issue_with_actor(
        &self,
        issue: Issue,
        actor: ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        let new_issue = issue.clone();
        let (issue_id, version) = self.inner.add_issue(issue, &actor).await?;
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: new_issue,
            actor,
        });
        self.event_bus
            .emit_issue_created(issue_id.clone(), version, payload);
        Ok((issue_id, version))
    }

    pub async fn update_issue_with_actor(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_issue = self.inner.get_issue(id, false).await.ok().map(|v| v.item);
        let new_issue = issue.clone();
        let version = self.inner.update_issue(id, issue, &actor).await?;
        let payload = Arc::new(MutationPayload::Issue {
            old: old_issue,
            new: new_issue,
            actor,
        });
        self.event_bus
            .emit_issue_updated(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn delete_issue_with_actor(
        &self,
        id: &IssueId,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_issue = self.inner.get_issue(id, false).await.ok().map(|v| v.item);
        let version = self.inner.delete_issue(id, &actor).await?;
        let new_issue = self
            .inner
            .get_issue(id, true)
            .await
            .ok()
            .map(|v| v.item)
            .or_else(|| old_issue.clone());
        let payload = Arc::new(MutationPayload::Issue {
            old: old_issue,
            new: new_issue.expect("entity must exist after successful delete"),
            actor,
        });
        self.event_bus
            .emit_issue_deleted(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn add_patch_with_actor(
        &self,
        patch: Patch,
        actor: ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        let new_patch = patch.clone();
        let (patch_id, version) = self.inner.add_patch(patch, &actor).await?;
        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: new_patch,
            actor,
        });
        self.event_bus
            .emit_patch_created(patch_id.clone(), version, payload);
        Ok((patch_id, version))
    }

    pub async fn update_patch_with_actor(
        &self,
        id: &PatchId,
        patch: Patch,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_patch = self.inner.get_patch(id, false).await.ok().map(|v| v.item);
        let new_patch = patch.clone();
        let version = self.inner.update_patch(id, patch, &actor).await?;
        let payload = Arc::new(MutationPayload::Patch {
            old: old_patch,
            new: new_patch,
            actor,
        });
        self.event_bus
            .emit_patch_updated(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn delete_patch_with_actor(
        &self,
        id: &PatchId,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_patch = self.inner.get_patch(id, false).await.ok().map(|v| v.item);
        let version = self.inner.delete_patch(id, &actor).await?;
        let new_patch = self
            .inner
            .get_patch(id, true)
            .await
            .ok()
            .map(|v| v.item)
            .or_else(|| old_patch.clone());
        let payload = Arc::new(MutationPayload::Patch {
            old: old_patch,
            new: new_patch.expect("entity must exist after successful delete"),
            actor,
        });
        self.event_bus
            .emit_patch_deleted(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn add_document_with_actor(
        &self,
        document: Document,
        actor: ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        let new_document = document.clone();
        let (document_id, version) = self.inner.add_document(document, &actor).await?;
        let payload = Arc::new(MutationPayload::Document {
            old: None,
            new: new_document,
            actor,
        });
        self.event_bus
            .emit_document_created(document_id.clone(), version, payload);
        Ok((document_id, version))
    }

    pub async fn update_document_with_actor(
        &self,
        id: &DocumentId,
        document: Document,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_document = self
            .inner
            .get_document(id, false)
            .await
            .ok()
            .map(|v| v.item);
        let new_document = document.clone();
        let version = self.inner.update_document(id, document, &actor).await?;
        let payload = Arc::new(MutationPayload::Document {
            old: old_document,
            new: new_document,
            actor,
        });
        self.event_bus
            .emit_document_updated(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn delete_document_with_actor(
        &self,
        id: &DocumentId,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_document = self
            .inner
            .get_document(id, false)
            .await
            .ok()
            .map(|v| v.item);
        let version = self.inner.delete_document(id, &actor).await?;
        let new_document = self
            .inner
            .get_document(id, true)
            .await
            .ok()
            .map(|v| v.item)
            .or_else(|| old_document.clone());
        let payload = Arc::new(MutationPayload::Document {
            old: old_document,
            new: new_document.expect("entity must exist after successful delete"),
            actor,
        });
        self.event_bus
            .emit_document_deleted(id.clone(), version, payload);
        Ok(version)
    }

    pub async fn add_task_with_actor(
        &self,
        task: Task,
        creation_time: DateTime<Utc>,
        actor: ActorRef,
    ) -> Result<(TaskId, VersionNumber), StoreError> {
        let new_task = task.clone();
        let (task_id, version) = self.inner.add_task(task, creation_time, &actor).await?;
        let payload = Arc::new(MutationPayload::Job {
            old: None,
            new: new_task,
            actor,
        });
        self.event_bus
            .emit_job_created(task_id.clone(), version, payload);
        Ok((task_id, version))
    }

    pub async fn update_task_with_actor(
        &self,
        metis_id: &TaskId,
        task: Task,
        actor: ActorRef,
    ) -> Result<Versioned<Task>, StoreError> {
        let old_task = self
            .inner
            .get_task(metis_id, false)
            .await
            .ok()
            .map(|v| v.item);
        let new_task = task.clone();
        let result = self.inner.update_task(metis_id, task, &actor).await?;
        let payload = Arc::new(MutationPayload::Job {
            old: old_task,
            new: new_task,
            actor,
        });
        self.event_bus
            .emit_job_updated(metis_id.clone(), result.version, payload);
        Ok(result)
    }

    pub async fn delete_task_with_actor(
        &self,
        id: &TaskId,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.inner.delete_task(id, &actor).await
    }

    // ---- Repository mutations (inherent, with actor) ----

    pub async fn add_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.inner.add_repository(name, config, &actor).await
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.inner.update_repository(name, config, &actor).await
    }

    pub async fn delete_repository(
        &self,
        name: &RepoName,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.inner.delete_repository(name, &actor).await
    }

    // ---- Actor mutations (inherent, with actor) ----

    pub async fn add_actor(&self, actor: Actor, acting_as: ActorRef) -> Result<(), StoreError> {
        self.inner.add_actor(actor, &acting_as).await
    }

    pub async fn update_actor(&self, actor: Actor, acting_as: ActorRef) -> Result<(), StoreError> {
        self.inner.update_actor(actor, &acting_as).await
    }

    // ---- User mutations (inherent, with actor) ----

    pub async fn add_user(&self, user: User, actor: ActorRef) -> Result<(), StoreError> {
        self.inner.add_user(user, &actor).await
    }

    pub async fn update_user(
        &self,
        user: User,
        actor: ActorRef,
    ) -> Result<Versioned<User>, StoreError> {
        self.inner.update_user(user, &actor).await
    }

    pub async fn delete_user(
        &self,
        username: &Username,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.inner.delete_user(username, &actor).await
    }

    // ---- Message mutations (inherent, with actor) ----

    pub async fn add_message_with_actor(
        &self,
        message: Message,
        actor: ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        let new_message = message.clone();
        let recipient = message.recipient.clone();
        let sender = message.sender.clone();
        let (message_id, version) = self.inner.add_message(message, &actor).await?;
        let payload = Arc::new(MutationPayload::Message {
            old: None,
            new: new_message,
            actor,
        });
        self.event_bus.emit_message_created(
            message_id.clone(),
            recipient,
            sender,
            version,
            payload,
        );
        Ok((message_id, version))
    }

    pub async fn update_message_with_actor(
        &self,
        id: &MessageId,
        message: Message,
        actor: ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let old_message = self.inner.get_message(id).await.ok().map(|v| v.item);
        let new_message = message.clone();
        let recipient = message.recipient.clone();
        let sender = message.sender.clone();
        let version = self.inner.update_message(id, message, &actor).await?;
        let payload = Arc::new(MutationPayload::Message {
            old: old_message,
            new: new_message,
            actor,
        });
        self.event_bus
            .emit_message_updated(id.clone(), recipient, sender, version, payload);
        Ok(version)
    }

    // ---- Notification mutations ----

    pub async fn insert_notification_with_actor(
        &self,
        notification: Notification,
        actor: ActorRef,
    ) -> Result<NotificationId, StoreError> {
        let new_notification = notification.clone();
        let notification_id = self.inner.insert_notification(notification).await?;
        let payload = Arc::new(MutationPayload::Notification {
            old: None,
            new: new_notification,
            actor,
        });
        self.event_bus
            .emit_notification_created(notification_id.clone(), 1, payload);
        Ok(notification_id)
    }

    pub async fn mark_notification_read(&self, id: &NotificationId) -> Result<(), StoreError> {
        self.inner.mark_notification_read(id).await
    }

    pub async fn mark_all_notifications_read(
        &self,
        recipient: &ActorId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError> {
        self.inner
            .mark_all_notifications_read(recipient, before)
            .await
    }

    // ---- Agent mutations ----

    pub async fn add_agent(&self, agent: Agent) -> Result<(), StoreError> {
        self.inner.add_agent(agent).await
    }

    pub async fn update_agent(&self, agent: Agent) -> Result<(), StoreError> {
        self.inner.update_agent(agent).await
    }

    pub async fn delete_agent(&self, name: &str) -> Result<(), StoreError> {
        self.inner.delete_agent(name).await
    }

    // ---- Label mutations ----

    pub async fn add_label(&self, label: Label, actor: ActorRef) -> Result<LabelId, StoreError> {
        let new_label = label.clone();
        let label_id = self.inner.add_label(label).await?;
        let payload = Arc::new(MutationPayload::Label {
            old: None,
            new: new_label,
            actor,
        });
        self.event_bus
            .emit_label_created(label_id.clone(), 1, payload);
        Ok(label_id)
    }

    pub async fn update_label(
        &self,
        id: &LabelId,
        label: Label,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        let old_label = self.inner.get_label(id).await.ok();
        let new_label = label.clone();
        self.inner.update_label(id, label).await?;
        let payload = Arc::new(MutationPayload::Label {
            old: old_label,
            new: new_label,
            actor,
        });
        self.event_bus.emit_label_updated(id.clone(), 1, payload);
        Ok(())
    }

    pub async fn delete_label(&self, id: &LabelId, actor: ActorRef) -> Result<(), StoreError> {
        let old_label = self.inner.get_label(id).await.ok();
        self.inner.delete_label(id).await?;
        // After soft-delete, get_label returns NotFound, so construct the
        // deleted version from the old label.
        let mut deleted_label = old_label
            .clone()
            .expect("entity must exist after successful delete");
        deleted_label.deleted = true;
        let payload = Arc::new(MutationPayload::Label {
            old: old_label,
            new: deleted_label,
            actor,
        });
        self.event_bus.emit_label_deleted(id.clone(), 1, payload);
        Ok(())
    }

    // ---- Label association mutations ----

    pub async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let changed = self
            .inner
            .add_label_association(label_id, object_id)
            .await?;
        if changed {
            self.emit_entity_updated_on_label_change(object_id).await;
        }
        Ok(changed)
    }

    pub async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let changed = self
            .inner
            .remove_label_association(label_id, object_id)
            .await?;
        if changed {
            self.emit_entity_updated_on_label_change(object_id).await;
        }
        Ok(changed)
    }

    // ---- Object relationship mutations ----

    pub async fn add_relationship_with_actor(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: RelationshipType,
        actor: ActorRef,
    ) -> Result<bool, StoreError> {
        let changed = self
            .inner
            .add_relationship(source_id, target_id, rel_type)
            .await?;
        if changed {
            self.emit_issue_updated_for_relationship(source_id, target_id, actor)
                .await;
        }
        Ok(changed)
    }

    pub async fn remove_relationship_with_actor(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: RelationshipType,
        actor: ActorRef,
    ) -> Result<bool, StoreError> {
        let changed = self
            .inner
            .remove_relationship(source_id, target_id, rel_type)
            .await?;
        if changed {
            self.emit_issue_updated_for_relationship(source_id, target_id, actor)
                .await;
        }
        Ok(changed)
    }

    /// For each side of the relationship that is an Issue, re-fetch the issue
    /// and emit an `IssueUpdated` event so SSE subscribers see the change.
    async fn emit_issue_updated_for_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        actor: ActorRef,
    ) {
        for metis_id in [source_id, target_id] {
            if let Some(issue_id) = metis_id.as_issue_id() {
                if let Ok(versioned) = self.inner.get_issue(&issue_id, false).await {
                    let payload = Arc::new(MutationPayload::Issue {
                        old: Some(versioned.item.clone()),
                        new: versioned.item,
                        actor: actor.clone(),
                    });
                    self.event_bus
                        .emit_issue_updated(issue_id, versioned.version, payload);
                }
            }
        }
    }

    // ---- User secret mutations ----

    pub async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
    ) -> Result<(), StoreError> {
        self.inner
            .set_user_secret(username, secret_name, encrypted_value)
            .await
    }

    pub async fn delete_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<(), StoreError> {
        self.inner.delete_user_secret(username, secret_name).await
    }

    /// Re-fetch the entity identified by `object_id` and emit an appropriate
    /// updated event so SSE subscribers see the label change.
    async fn emit_entity_updated_on_label_change(&self, object_id: &MetisId) {
        let label_actor = ActorRef::System {
            worker_name: "label-association".into(),
            on_behalf_of: None,
        };
        if let Some(issue_id) = object_id.as_issue_id() {
            if let Ok(versioned) = self.inner.get_issue(&issue_id, false).await {
                let payload = Arc::new(MutationPayload::Issue {
                    old: Some(versioned.item.clone()),
                    new: versioned.item,
                    actor: label_actor,
                });
                self.event_bus
                    .emit_issue_updated(issue_id, versioned.version, payload);
            }
        } else if let Some(patch_id) = object_id.as_patch_id() {
            if let Ok(versioned) = self.inner.get_patch(&patch_id, false).await {
                let payload = Arc::new(MutationPayload::Patch {
                    old: Some(versioned.item.clone()),
                    new: versioned.item,
                    actor: label_actor,
                });
                self.event_bus
                    .emit_patch_updated(patch_id, versioned.version, payload);
            }
        } else if let Some(doc_id) = object_id.as_document_id() {
            if let Ok(versioned) = self.inner.get_document(&doc_id, false).await {
                let payload = Arc::new(MutationPayload::Document {
                    old: Some(versioned.item.clone()),
                    new: versioned.item,
                    actor: label_actor,
                });
                self.event_bus
                    .emit_document_updated(doc_id, versioned.version, payload);
            }
        }
    }
}

#[async_trait]
impl ReadOnlyStore for StoreWithEvents {
    // ---- Repository (read-only) ----

    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        self.inner.get_repository(name, include_deleted).await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        self.inner.list_repositories(query).await
    }

    // ---- Issue (read-only) ----

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

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        self.inner.list_issues(query).await
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        self.inner.search_issue_graph(filters).await
    }

    // ---- Issue graph queries (read-only) ----

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_children(issue_id).await
    }

    async fn get_issue_subtrees(
        &self,
        root_ids: &[IssueId],
    ) -> Result<Vec<metis_common::api::v1::issues::SubtreeIssueRow>, StoreError> {
        self.inner.get_issue_subtrees(root_ids).await
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_blocked_on(issue_id).await
    }

    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        self.inner.get_tasks_for_issue(issue_id).await
    }

    // ---- Patch (read-only) ----

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

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        self.inner.list_patches(query).await
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issues_for_patch(patch_id).await
    }

    // ---- Document (read-only) ----

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

    // ---- Task/Job (read-only) ----

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

    async fn get_jobs_summary_for_issues(
        &self,
        issue_ids: &[IssueId],
    ) -> Result<
        HashMap<IssueId, metis_common::api::v1::issues::JobStatusSummary>,
        StoreError,
    > {
        self.inner.get_jobs_summary_for_issues(issue_ids).await
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

    // ---- Actor (read-only) ----

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        self.inner.get_actor(name).await
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        self.inner.list_actors().await
    }

    // ---- User (read-only) ----

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

    // ---- Message (read-only) ----

    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        self.inner.get_message(id).await
    }

    async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        self.inner.list_messages(query).await
    }

    // ---- Notification (read-only) ----

    async fn get_notification(&self, id: &NotificationId) -> Result<Notification, StoreError> {
        self.inner.get_notification(id).await
    }

    async fn list_notifications(
        &self,
        query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError> {
        self.inner.list_notifications(query).await
    }

    async fn count_unread_notifications(&self, recipient: &ActorId) -> Result<u64, StoreError> {
        self.inner.count_unread_notifications(recipient).await
    }

    // ---- Agent (read-only) ----

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        self.inner.get_agent(name).await
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        self.inner.list_agents().await
    }

    // ---- Label (read-only) ----

    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError> {
        self.inner.get_label(id).await
    }

    async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        self.inner.list_labels(query).await
    }

    async fn get_label_by_name(&self, name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        self.inner.get_label_by_name(name).await
    }

    // ---- Label association (read-only) ----

    async fn get_labels_for_object(
        &self,
        object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        self.inner.get_labels_for_object(object_id).await
    }

    async fn get_labels_for_objects(
        &self,
        object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        self.inner.get_labels_for_objects(object_ids).await
    }

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<MetisId>, StoreError> {
        self.inner.get_objects_for_label(label_id).await
    }

    // ---- Object relationships (read-only) ----

    async fn get_relationships(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        self.inner
            .get_relationships(source_id, target_id, rel_type)
            .await
    }

    // ---- User secrets (read-only) ----

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.inner.get_user_secret(username, secret_name).await
    }

    async fn list_user_secret_names(&self, username: &Username) -> Result<Vec<String>, StoreError> {
        self.inner.list_user_secret_names(username).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemoryStore, Status};

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

    fn dummy_issue() -> Issue {
        use crate::domain::issues::{IssueStatus, IssueType};
        use crate::domain::users::Username;

        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn subscribe_receives_emitted_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue_id = IssueId::new();
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: dummy_issue(),
            actor: ActorRef::test(),
        });
        bus.emit_issue_created(issue_id.clone(), 1, payload);

        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.seq(), 1);
        match event {
            ServerEvent::IssueCreated {
                issue_id: id,
                seq,
                version,
                ..
            } => {
                assert_eq!(id, issue_id);
                assert_eq!(seq, 1);
                assert_eq!(version, 1);
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_arrive_in_order() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue = dummy_issue();
        let issue_id = IssueId::new();
        let payload1 = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue.clone(),
            actor: ActorRef::test(),
        });
        bus.emit_issue_created(issue_id.clone(), 1, payload1);
        let payload2 = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: ActorRef::test(),
        });
        bus.emit_issue_updated(issue_id, 2, payload2);

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
            "Test Title".to_string(),
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

        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueCreated");
        match &event {
            ServerEvent::IssueCreated {
                issue_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, issue_id);
                assert_eq!(*version, 1);
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
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
            "Test Title".to_string(),
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

        let (issue_id, _) = store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let mut updated = issue;
        updated.status = IssueStatus::InProgress;
        store
            .update_issue_with_actor(&issue_id, updated, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueUpdated");
        match &event {
            ServerEvent::IssueUpdated {
                issue_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, issue_id);
                assert_eq!(*version, 2);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
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
            "Test Title".to_string(),
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

        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueDeleted");
        match &event {
            ServerEvent::IssueDeleted {
                issue_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, issue_id);
                assert_eq!(*version, 2);
            }
            other => panic!("expected IssueDeleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_event_carries_new_entity_payload() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "payload test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueCreated");
        match &event {
            ServerEvent::IssueCreated { payload, .. } => match payload.as_ref() {
                MutationPayload::Issue { old, new, .. } => {
                    assert!(old.is_none(), "create event should have no old state");
                    assert_eq!(new.description, "payload test");
                    assert_eq!(new.status, IssueStatus::Open);
                }
                other => panic!("expected Issue payload, got {other:?}"),
            },
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_event_carries_old_and_new_entity_payload() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "before update".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let (issue_id, _) = store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let mut updated = issue;
        updated.status = IssueStatus::InProgress;
        updated.description = "after update".to_string();
        store
            .update_issue_with_actor(&issue_id, updated, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueUpdated");
        match &event {
            ServerEvent::IssueUpdated { payload, .. } => match payload.as_ref() {
                MutationPayload::Issue { old, new, .. } => {
                    let old = old.as_ref().expect("update event should carry old state");
                    assert_eq!(old.status, IssueStatus::Open);
                    assert_eq!(old.description, "before update");
                    assert_eq!(new.status, IssueStatus::InProgress);
                    assert_eq!(new.description, "after update");
                }
                other => panic!("expected Issue payload, got {other:?}"),
            },
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delete_event_carries_old_entity_payload() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "to be deleted".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueDeleted");
        match &event {
            ServerEvent::IssueDeleted { payload, .. } => match payload.as_ref() {
                MutationPayload::Issue { old, new, .. } => {
                    let old = old.as_ref().expect("delete event should carry old state");
                    assert_eq!(old.description, "to be deleted");
                    assert!(!old.deleted, "old state should not be deleted");
                    assert!(new.deleted, "new state should be soft-deleted");
                }
                other => panic!("expected Issue payload, got {other:?}"),
            },
            other => panic!("expected IssueDeleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn job_update_event_carries_old_and_new_payload() {
        use crate::store::Task as StoreTask;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let task = StoreTask {
            prompt: "test task".to_string(),
            context: crate::domain::jobs::BundleSpec::None,
            spawned_from: None,
            creator: crate::domain::users::Username::from("test-user"),
            image: None,
            model: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            status: Status::Created,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
        };

        let (task_id, _) = store
            .add_task_with_actor(task, chrono::Utc::now(), ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume JobCreated

        let updated_task = StoreTask {
            prompt: "test task".to_string(),
            context: crate::domain::jobs::BundleSpec::None,
            spawned_from: None,
            creator: crate::domain::users::Username::from("test-user"),
            image: None,
            model: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            status: Status::Running,
            last_message: Some("doing work".to_string()),
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
        };

        store
            .update_task_with_actor(&task_id, updated_task, ActorRef::test())
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive JobUpdated");
        match &event {
            ServerEvent::JobUpdated { payload, .. } => match payload.as_ref() {
                MutationPayload::Job { old, new, .. } => {
                    let old = old.as_ref().expect("update event should carry old state");
                    assert_eq!(old.status, Status::Created);
                    assert!(old.last_message.is_none());
                    assert_eq!(new.status, Status::Running);
                    assert_eq!(new.last_message.as_deref(), Some("doing work"));
                }
                other => panic!("expected Job payload, got {other:?}"),
            },
            other => panic!("expected JobUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn actor_context_carried_through_events() {
        use crate::domain::actors::ActorId;
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "actor test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        store
            .add_issue_with_actor(
                issue,
                ActorRef::Authenticated {
                    actor_id: ActorId::Username(Username::from("testuser").into()),
                },
            )
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueCreated");
        match &event {
            ServerEvent::IssueCreated { payload, .. } => {
                assert_eq!(
                    *payload.actor(),
                    ActorRef::Authenticated {
                        actor_id: ActorId::Username(Username::from("testuser").into())
                    },
                    "actor should be carried through the event payload"
                );
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn actor_context_preserves_system_variant() {
        use crate::domain::issues::{Issue, IssueStatus, IssueType};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let issue = Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "system actor test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        store
            .add_issue_with_actor(
                issue,
                ActorRef::System {
                    worker_name: "my_worker".into(),
                    on_behalf_of: None,
                },
            )
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueCreated");
        match &event {
            ServerEvent::IssueCreated { payload, .. } => {
                assert_eq!(
                    *payload.actor(),
                    ActorRef::System {
                        worker_name: "my_worker".into(),
                        on_behalf_of: None,
                    },
                    "actor should be preserved as System variant"
                );
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn store_with_events_emits_on_add_message() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let recipient = crate::domain::actors::ActorId::Issue(
            "i-abcdef".parse::<metis_common::IssueId>().unwrap(),
        );
        let sender = crate::domain::actors::ActorId::Username(
            crate::domain::users::Username::from("alice").into(),
        );
        let message = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "hello world".to_string(),
        );

        let (message_id, version) = store
            .add_message_with_actor(message, ActorRef::test())
            .await
            .unwrap();

        assert_eq!(version, 1);

        let event = rx.recv().await.expect("should receive MessageCreated");
        match &event {
            ServerEvent::MessageCreated {
                message_id: id,
                recipient: r,
                sender: s,
                version: v,
                payload,
                ..
            } => {
                assert_eq!(*id, message_id);
                assert_eq!(*r, recipient);
                assert_eq!(*s, Some(sender));
                assert_eq!(*v, 1);
                match payload.as_ref() {
                    MutationPayload::Message { old, new, .. } => {
                        assert!(old.is_none(), "create event should have no old state");
                        assert_eq!(new.body, "hello world");
                    }
                    other => panic!("expected Message payload, got {other:?}"),
                }
            }
            other => panic!("expected MessageCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn store_with_events_emits_on_update_message() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner, bus.clone());
        let mut rx = bus.subscribe();

        let recipient = crate::domain::actors::ActorId::Issue(
            "i-abcdef".parse::<metis_common::IssueId>().unwrap(),
        );
        let sender = crate::domain::actors::ActorId::Username(
            crate::domain::users::Username::from("alice").into(),
        );
        let message = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "original".to_string(),
        );

        let (message_id, _) = store
            .add_message_with_actor(message, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume MessageCreated

        let updated_message = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "updated".to_string(),
        );

        let version = store
            .update_message_with_actor(&message_id, updated_message, ActorRef::test())
            .await
            .unwrap();

        assert_eq!(version, 2);

        let event = rx.recv().await.expect("should receive MessageUpdated");
        match &event {
            ServerEvent::MessageUpdated {
                message_id: id,
                recipient: r,
                sender: s,
                version: v,
                payload,
                ..
            } => {
                assert_eq!(*id, message_id);
                assert_eq!(*r, recipient);
                assert_eq!(*s, Some(sender));
                assert_eq!(*v, 2);
                match payload.as_ref() {
                    MutationPayload::Message { old, new, .. } => {
                        let old = old.as_ref().expect("update event should carry old state");
                        assert_eq!(old.body, "original");
                        assert_eq!(new.body, "updated");
                    }
                    other => panic!("expected Message payload, got {other:?}"),
                }
            }
            other => panic!("expected MessageUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_label_association_emits_issue_updated() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue = dummy_issue();
        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let label = crate::domain::labels::Label::new(
            "bug".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(issue_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueUpdated");
        match &event {
            ServerEvent::IssueUpdated {
                issue_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, issue_id);
                assert_eq!(*version, 1);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_label_association_twice_emits_only_one_event() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue = dummy_issue();
        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let label = crate::domain::labels::Label::new(
            "bug".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(issue_id.clone());

        // First add should return true and emit an event
        let changed = store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        assert!(changed);
        let event = rx.recv().await.expect("should receive IssueUpdated");
        assert!(matches!(event, ServerEvent::IssueUpdated { .. }));

        // Second add (duplicate) should return false and NOT emit an event
        let changed = store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        assert!(!changed);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted for duplicate add"
        );
    }

    #[tokio::test]
    async fn remove_nonexistent_label_association_does_not_emit_event() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue = dummy_issue();
        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let label = crate::domain::labels::Label::new(
            "bug".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(issue_id.clone());

        // Removing a label that was never added should return false and not emit
        let changed = store
            .remove_label_association(&label_id, &object_id)
            .await
            .unwrap();
        assert!(!changed);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted for no-op remove"
        );
    }

    #[tokio::test]
    async fn remove_label_association_emits_issue_updated() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue = dummy_issue();
        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let label = crate::domain::labels::Label::new(
            "bug".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(issue_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueUpdated from add

        store
            .remove_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive IssueUpdated");
        match &event {
            ServerEvent::IssueUpdated {
                issue_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, issue_id);
                assert_eq!(*version, 1);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_label_association_emits_patch_updated() {
        use crate::domain::patches::{Patch, PatchStatus};
        use crate::domain::users::Username;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let patch = Patch::new(
            "Test patch".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("creator"),
            Vec::new(),
            "test/repo".parse().unwrap(),
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = store
            .add_patch_with_actor(patch, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume PatchCreated

        let label = crate::domain::labels::Label::new(
            "urgent".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(patch_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive PatchUpdated");
        match &event {
            ServerEvent::PatchUpdated {
                patch_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, patch_id);
                assert_eq!(*version, 1);
            }
            other => panic!("expected PatchUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_label_association_emits_document_updated() {
        use crate::domain::documents::Document;

        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let doc = Document {
            title: "Test doc".to_string(),
            body_markdown: "content".to_string(),
            path: None,
            created_by: None,
            deleted: false,
        };
        let (doc_id, _) = store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume DocumentCreated

        let label = crate::domain::labels::Label::new(
            "docs".to_string(),
            "#3498db".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        let object_id = MetisId::from(doc_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let event = rx.recv().await.expect("should receive DocumentUpdated");
        match &event {
            ServerEvent::DocumentUpdated {
                document_id: id,
                version,
                ..
            } => {
                assert_eq!(*id, doc_id);
                assert_eq!(*version, 1);
            }
            other => panic!("expected DocumentUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn label_association_on_unknown_entity_does_not_emit_event() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let label = crate::domain::labels::Label::new(
            "tag".to_string(),
            "#000000".parse().unwrap(),
            true,
            false,
        );
        let label_id = inner.add_label(label).await.unwrap();

        // Use a patch-like id for a patch that doesn't exist in the store
        let object_id: MetisId = "p-fakepatch".parse().unwrap();
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        // No event should be emitted because the patch doesn't exist
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn add_relationship_with_actor_emits_issue_updated_for_both_sides() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue1 = dummy_issue();
        let (issue_id1, _) = store
            .add_issue_with_actor(issue1, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let issue2 = dummy_issue();
        let (issue_id2, _) = store
            .add_issue_with_actor(issue2, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueCreated

        let source_id = MetisId::from(issue_id1.clone());
        let target_id = MetisId::from(issue_id2.clone());

        let changed = store
            .add_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::ChildOf,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert!(changed);

        // Should receive IssueUpdated for the source issue
        let event = rx
            .recv()
            .await
            .expect("should receive IssueUpdated for source");
        match &event {
            ServerEvent::IssueUpdated { issue_id, .. } => {
                assert_eq!(*issue_id, issue_id1);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }

        // Should receive IssueUpdated for the target issue
        let event = rx
            .recv()
            .await
            .expect("should receive IssueUpdated for target");
        match &event {
            ServerEvent::IssueUpdated { issue_id, .. } => {
                assert_eq!(*issue_id, issue_id2);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_relationship_duplicate_does_not_emit_event() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue1 = dummy_issue();
        let (issue_id1, _) = store
            .add_issue_with_actor(issue1, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let issue2 = dummy_issue();
        let (issue_id2, _) = store
            .add_issue_with_actor(issue2, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let source_id = MetisId::from(issue_id1.clone());
        let target_id = MetisId::from(issue_id2.clone());

        // First add
        let changed = store
            .add_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::ChildOf,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert!(changed);
        let _ = rx.recv().await.unwrap(); // consume IssueUpdated (source)
        let _ = rx.recv().await.unwrap(); // consume IssueUpdated (target)

        // Second add (duplicate)
        let changed = store
            .add_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::ChildOf,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert!(!changed);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted for duplicate add"
        );
    }

    #[tokio::test]
    async fn remove_relationship_with_actor_emits_issue_updated_for_both_sides() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue1 = dummy_issue();
        let (issue_id1, _) = store
            .add_issue_with_actor(issue1, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let issue2 = dummy_issue();
        let (issue_id2, _) = store
            .add_issue_with_actor(issue2, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let source_id = MetisId::from(issue_id1.clone());
        let target_id = MetisId::from(issue_id2.clone());

        // Add relationship first
        store
            .add_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::BlockedOn,
                ActorRef::test(),
            )
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap(); // consume IssueUpdated (source)
        let _ = rx.recv().await.unwrap(); // consume IssueUpdated (target)

        // Now remove it
        let changed = store
            .remove_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::BlockedOn,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert!(changed);

        // Should receive IssueUpdated for the source issue
        let event = rx
            .recv()
            .await
            .expect("should receive IssueUpdated for source");
        match &event {
            ServerEvent::IssueUpdated { issue_id, .. } => {
                assert_eq!(*issue_id, issue_id1);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }

        // Should receive IssueUpdated for the target issue
        let event = rx
            .recv()
            .await
            .expect("should receive IssueUpdated for target");
        match &event {
            ServerEvent::IssueUpdated { issue_id, .. } => {
                assert_eq!(*issue_id, issue_id2);
            }
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_nonexistent_relationship_does_not_emit_event() {
        let bus = Arc::new(EventBus::new());
        let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let store = StoreWithEvents::new(inner.clone(), bus.clone());
        let mut rx = bus.subscribe();

        let issue1 = dummy_issue();
        let (issue_id1, _) = store
            .add_issue_with_actor(issue1, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let issue2 = dummy_issue();
        let (issue_id2, _) = store
            .add_issue_with_actor(issue2, ActorRef::test())
            .await
            .unwrap();
        let _ = rx.recv().await.unwrap();

        let source_id = MetisId::from(issue_id1.clone());
        let target_id = MetisId::from(issue_id2.clone());

        let changed = store
            .remove_relationship_with_actor(
                &source_id,
                &target_id,
                crate::store::RelationshipType::ChildOf,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert!(!changed);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted for no-op remove"
        );
    }
}
