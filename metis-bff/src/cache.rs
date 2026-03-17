use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use futures::StreamExt;
use tracing::{debug, error, info, warn};

use metis::client::MetisClient;
use metis_common::api::v1::events::{
    ConnectedEventData, EntityEventData, EventsQuery, HeartbeatEventData, ResyncEventData,
    SseEventType,
};
use metis_common::api::v1::messages::VersionedMessage;
use metis_common::documents::DocumentSummaryRecord;
use metis_common::issues::IssueSummaryRecord;
use metis_common::labels::LabelSummary;
use metis_common::notifications::NotificationResponse;
use metis_common::patches::PatchSummaryRecord;
use metis_common::sessions::SessionSummaryRecord;
use metis_common::{DocumentId, IssueId, LabelId, MessageId, NotificationId, PatchId, SessionId};

/// In-memory entity cache populated by the upstream SSE event stream.
///
/// The cache stores Summary variants of entities where available to reduce
/// memory usage. It is populated by a background task and is passive -- it
/// does not serve requests directly (that is future work).
pub struct EntityCache {
    pub issues: DashMap<IssueId, IssueSummaryRecord>,
    pub patches: DashMap<PatchId, PatchSummaryRecord>,
    pub sessions: DashMap<SessionId, SessionSummaryRecord>,
    pub documents: DashMap<DocumentId, DocumentSummaryRecord>,
    pub labels: DashMap<LabelId, LabelSummary>,
    pub notifications: DashMap<NotificationId, NotificationResponse>,
    pub messages: DashMap<MessageId, VersionedMessage>,

    last_event_id: AtomicU64,
    ready: AtomicBool,
}

impl EntityCache {
    pub fn new() -> Self {
        Self {
            issues: DashMap::new(),
            patches: DashMap::new(),
            sessions: DashMap::new(),
            documents: DashMap::new(),
            labels: DashMap::new(),
            notifications: DashMap::new(),
            messages: DashMap::new(),
            last_event_id: AtomicU64::new(0),
            ready: AtomicBool::new(false),
        }
    }

    /// Whether the cache has received the initial connected event.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// The last SSE event ID processed, for reconnection.
    pub fn last_event_id(&self) -> u64 {
        self.last_event_id.load(Ordering::Acquire)
    }

    /// Clear all cached data and reset readiness state.
    pub fn clear(&self) {
        self.issues.clear();
        self.patches.clear();
        self.sessions.clear();
        self.documents.clear();
        self.labels.clear();
        self.notifications.clear();
        self.messages.clear();
        self.ready.store(false, Ordering::Release);
    }

    /// Apply a single SSE event to the cache, updating or removing entities.
    pub fn apply_event(
        &self,
        event_type: &SseEventType,
        event_id: Option<u64>,
        data: &str,
    ) -> Result<(), CacheError> {
        match event_type {
            SseEventType::Connected => {
                let connected: ConnectedEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("connected: {e}")))?;
                debug!(
                    current_seq = connected.current_seq,
                    "received connected event"
                );
            }
            SseEventType::Resync => {
                let resync: ResyncEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("resync: {e}")))?;
                warn!(
                    reason = %resync.reason,
                    current_seq = resync.current_seq,
                    "received resync event, clearing cache"
                );
                self.clear();
            }
            SseEventType::Heartbeat => {
                let _heartbeat: HeartbeatEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("heartbeat: {e}")))?;
                debug!("received heartbeat");
            }

            // Issue events
            SseEventType::IssueCreated | SseEventType::IssueUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("issue event: {e}")))?;
                let record: IssueSummaryRecord = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("issue entity: {e}")))?;
                self.issues.insert(record.issue_id.clone(), record);
            }
            SseEventType::IssueDeleted => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("issue delete: {e}")))?;
                let id: IssueId =
                    event
                        .entity_id
                        .parse()
                        .map_err(|e: metis_common::MetisIdError| {
                            CacheError::Parse(format!("issue id: {e}"))
                        })?;
                self.issues.remove(&id);
            }

            // Patch events
            SseEventType::PatchCreated | SseEventType::PatchUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("patch event: {e}")))?;
                let record: PatchSummaryRecord = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("patch entity: {e}")))?;
                self.patches.insert(record.patch_id.clone(), record);
            }
            SseEventType::PatchDeleted => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("patch delete: {e}")))?;
                let id: PatchId =
                    event
                        .entity_id
                        .parse()
                        .map_err(|e: metis_common::MetisIdError| {
                            CacheError::Parse(format!("patch id: {e}"))
                        })?;
                self.patches.remove(&id);
            }

            // Session events
            SseEventType::SessionCreated | SseEventType::SessionUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("session event: {e}")))?;
                let record: SessionSummaryRecord = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("session entity: {e}")))?;
                self.sessions.insert(record.session_id.clone(), record);
            }

            // Document events
            SseEventType::DocumentCreated | SseEventType::DocumentUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("document event: {e}")))?;
                let record: DocumentSummaryRecord = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("document entity: {e}")))?;
                self.documents.insert(record.document_id.clone(), record);
            }
            SseEventType::DocumentDeleted => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("document delete: {e}")))?;
                let id: DocumentId =
                    event
                        .entity_id
                        .parse()
                        .map_err(|e: metis_common::MetisIdError| {
                            CacheError::Parse(format!("document id: {e}"))
                        })?;
                self.documents.remove(&id);
            }

            // Label events
            SseEventType::LabelCreated | SseEventType::LabelUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("label event: {e}")))?;
                let record: LabelSummary = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("label entity: {e}")))?;
                self.labels.insert(record.label_id.clone(), record);
            }
            SseEventType::LabelDeleted => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("label delete: {e}")))?;
                let id: LabelId =
                    event
                        .entity_id
                        .parse()
                        .map_err(|e: metis_common::MetisIdError| {
                            CacheError::Parse(format!("label id: {e}"))
                        })?;
                self.labels.remove(&id);
            }

            // Message events
            SseEventType::MessageCreated | SseEventType::MessageUpdated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("message event: {e}")))?;
                let record: VersionedMessage = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("message entity: {e}")))?;
                self.messages.insert(record.message_id.clone(), record);
            }

            // Notification events
            SseEventType::NotificationCreated => {
                let event: EntityEventData = serde_json::from_str(data)
                    .map_err(|e| CacheError::Parse(format!("notification event: {e}")))?;
                let record: NotificationResponse = serde_json::from_value(event.entity.clone())
                    .map_err(|e| CacheError::Parse(format!("notification entity: {e}")))?;
                self.notifications
                    .insert(record.notification_id.clone(), record);
            }
        }

        // Update last event ID if provided.
        if let Some(id) = event_id {
            self.last_event_id.store(id, Ordering::Release);
        }

        Ok(())
    }

    fn set_ready(&self) {
        self.ready.store(true, Ordering::Release);
        info!("entity cache is ready");
    }
}

impl Default for EntityCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum CacheError {
    Parse(String),
    Upstream(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Parse(msg) => write!(f, "cache parse error: {msg}"),
            CacheError::Upstream(msg) => write!(f, "cache upstream error: {msg}"),
        }
    }
}

impl std::error::Error for CacheError {}

/// Spawn a background task that connects to the upstream SSE `/v1/events`
/// endpoint via MetisClient and populates the entity cache with incremental
/// updates.
///
/// The task will:
/// 1. Connect to the SSE stream using MetisClient::subscribe_events
/// 2. Process the initial connected event
/// 3. Apply incremental updates from the stream
/// 4. Reconnect with Last-Event-ID on disconnect
pub fn spawn_cache_population_task(
    cache: Arc<EntityCache>,
    client: MetisClient,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            info!("connecting to SSE stream for cache population");

            match run_sse_loop(&cache, &client).await {
                Ok(()) => {
                    info!("SSE stream ended normally, reconnecting");
                }
                Err(e) => {
                    error!(error = %e, "SSE cache population error, reconnecting in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    })
}

async fn run_sse_loop(cache: &EntityCache, client: &MetisClient) -> Result<(), CacheError> {
    let last_event_id = cache.last_event_id();
    let last_id = if last_event_id > 0 {
        Some(last_event_id)
    } else {
        None
    };

    let query = EventsQuery::default();
    let mut stream = client
        .subscribe_events(&query, last_id)
        .await
        .map_err(|e| CacheError::Upstream(format!("SSE connection failed: {e}")))?;

    let mut received_connected = false;

    while let Some(event_result) = stream.next().await {
        let event =
            event_result.map_err(|e| CacheError::Upstream(format!("SSE stream error: {e}")))?;

        if let Err(e) = cache.apply_event(&event.event_type, event.id, &event.data) {
            warn!(event_type = ?event.event_type, error = %e, "failed to apply SSE event to cache");
        }

        // Mark ready after first connected event
        if event.event_type == SseEventType::Connected && !received_connected {
            received_connected = true;
            cache.set_ready();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_issue_event(issue_id: &str, version: u64) -> String {
        serde_json::json!({
            "entity_type": "issue",
            "entity_id": issue_id,
            "version": version,
            "timestamp": "2026-01-01T00:00:00Z",
            "entity": {
                "issue_id": issue_id,
                "version": version,
                "timestamp": "2026-01-01T00:00:00Z",
                "issue": {
                    "type": "task",
                    "title": "Test issue",
                    "description": "A test issue",
                    "creator": "testuser",
                    "status": "open",
                    "assignee": null,
                    "progress": "",
                    "dependencies": [],
                    "patches": [],
                    "todo_list": [],
                    "deleted": false,
                    "labels": []
                },
                "actor": null,
                "creation_time": "2026-01-01T00:00:00Z"
            }
        })
        .to_string()
    }

    fn make_patch_event(patch_id: &str, version: u64) -> String {
        serde_json::json!({
            "entity_type": "patch",
            "entity_id": patch_id,
            "version": version,
            "timestamp": "2026-01-01T00:00:00Z",
            "entity": {
                "patch_id": patch_id,
                "version": version,
                "timestamp": "2026-01-01T00:00:00Z",
                "patch": {
                    "title": "Test patch",
                    "status": "open",
                    "is_automatic_backup": false,
                    "created_by": null,
                    "creator": "testuser",
                    "review_summary": { "count": 0, "approved": false },
                    "service_repo_name": "testowner/testrepo",
                    "github": null,
                    "branch_name": null,
                    "base_branch": null,
                    "deleted": false,
                    "labels": []
                },
                "actor": null,
                "creation_time": "2026-01-01T00:00:00Z"
            }
        })
        .to_string()
    }

    fn make_session_event(session_id: &str, version: u64) -> String {
        serde_json::json!({
            "entity_type": "session",
            "entity_id": session_id,
            "version": version,
            "timestamp": "2026-01-01T00:00:00Z",
            "entity": {
                "session_id": session_id,
                "version": version,
                "timestamp": "2026-01-01T00:00:00Z",
                "session": {
                    "prompt": "Test prompt...",
                    "spawned_from": null,
                    "creator": "testuser",
                    "status": "running",
                    "error": null,
                    "deleted": false,
                    "creation_time": "2026-01-01T00:00:00Z",
                    "start_time": null,
                    "end_time": null
                },
                "actor": null
            }
        })
        .to_string()
    }

    fn make_delete_event(entity_type: &str, entity_id: &str) -> String {
        serde_json::json!({
            "entity_type": entity_type,
            "entity_id": entity_id,
            "version": 2,
            "timestamp": "2026-01-01T00:00:00Z",
            "entity": null
        })
        .to_string()
    }

    #[test]
    fn test_cache_new_is_not_ready() {
        let cache = EntityCache::new();
        assert!(!cache.is_ready());
        assert_eq!(cache.last_event_id(), 0);
    }

    #[test]
    fn test_apply_issue_created() {
        let cache = EntityCache::new();
        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &data)
            .unwrap();

        assert_eq!(cache.issues.len(), 1);
        let id: IssueId = "i-abcdef".parse().unwrap();
        assert!(cache.issues.contains_key(&id));
        assert_eq!(cache.last_event_id(), 1);
    }

    #[test]
    fn test_apply_issue_updated() {
        let cache = EntityCache::new();

        // Create then update
        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &data)
            .unwrap();

        let data = make_issue_event("i-abcdef", 2);
        cache
            .apply_event(&SseEventType::IssueUpdated, Some(2), &data)
            .unwrap();

        assert_eq!(cache.issues.len(), 1);
        let id: IssueId = "i-abcdef".parse().unwrap();
        let record = cache.issues.get(&id).unwrap();
        assert_eq!(record.version, 2);
        assert_eq!(cache.last_event_id(), 2);
    }

    #[test]
    fn test_apply_issue_deleted() {
        let cache = EntityCache::new();

        // Create then delete
        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &data)
            .unwrap();
        assert_eq!(cache.issues.len(), 1);

        let data = make_delete_event("issue", "i-abcdef");
        cache
            .apply_event(&SseEventType::IssueDeleted, Some(2), &data)
            .unwrap();

        assert_eq!(cache.issues.len(), 0);
    }

    #[test]
    fn test_apply_patch_created() {
        let cache = EntityCache::new();
        let data = make_patch_event("p-ghijkl", 1);
        cache
            .apply_event(&SseEventType::PatchCreated, Some(3), &data)
            .unwrap();

        assert_eq!(cache.patches.len(), 1);
        let id: PatchId = "p-ghijkl".parse().unwrap();
        assert!(cache.patches.contains_key(&id));
    }

    #[test]
    fn test_apply_patch_deleted() {
        let cache = EntityCache::new();

        let data = make_patch_event("p-ghijkl", 1);
        cache
            .apply_event(&SseEventType::PatchCreated, Some(1), &data)
            .unwrap();

        let data = make_delete_event("patch", "p-ghijkl");
        cache
            .apply_event(&SseEventType::PatchDeleted, Some(2), &data)
            .unwrap();

        assert_eq!(cache.patches.len(), 0);
    }

    #[test]
    fn test_apply_session_created() {
        let cache = EntityCache::new();
        let data = make_session_event("s-mnopqr", 1);
        cache
            .apply_event(&SseEventType::SessionCreated, Some(4), &data)
            .unwrap();

        assert_eq!(cache.sessions.len(), 1);
        let id: SessionId = "s-mnopqr".parse().unwrap();
        assert!(cache.sessions.contains_key(&id));
    }

    #[test]
    fn test_apply_connected_does_not_set_ready() {
        let cache = EntityCache::new();
        let data = serde_json::json!({
            "current_seq": 42
        })
        .to_string();

        cache
            .apply_event(&SseEventType::Connected, Some(0), &data)
            .unwrap();

        // apply_event doesn't set ready -- that's done by the population task
        assert!(!cache.is_ready());
    }

    #[test]
    fn test_apply_resync_clears_cache() {
        let cache = EntityCache::new();

        // Populate some data
        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &data)
            .unwrap();
        cache.set_ready();
        assert!(cache.is_ready());

        // Resync should clear everything
        let data = serde_json::json!({
            "reason": "client fell behind",
            "current_seq": 100
        })
        .to_string();
        cache
            .apply_event(&SseEventType::Resync, Some(100), &data)
            .unwrap();

        assert_eq!(cache.issues.len(), 0);
        assert!(!cache.is_ready());
    }

    #[test]
    fn test_apply_heartbeat() {
        let cache = EntityCache::new();
        let data = serde_json::json!({
            "server_time": "2026-01-01T00:00:00Z"
        })
        .to_string();

        cache
            .apply_event(&SseEventType::Heartbeat, Some(5), &data)
            .unwrap();

        assert_eq!(cache.last_event_id(), 5);
    }

    #[test]
    fn test_clear_resets_everything() {
        let cache = EntityCache::new();

        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &data)
            .unwrap();
        cache.set_ready();

        cache.clear();

        assert_eq!(cache.issues.len(), 0);
        assert_eq!(cache.patches.len(), 0);
        assert_eq!(cache.sessions.len(), 0);
        assert!(!cache.is_ready());
    }

    #[test]
    fn test_last_event_id_updates() {
        let cache = EntityCache::new();
        assert_eq!(cache.last_event_id(), 0);

        let data = make_issue_event("i-abcdef", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(42), &data)
            .unwrap();

        assert_eq!(cache.last_event_id(), 42);
    }

    #[test]
    fn test_parse_error_on_invalid_data() {
        let cache = EntityCache::new();
        let result = cache.apply_event(&SseEventType::IssueCreated, Some(1), "not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_entity_types() {
        let cache = EntityCache::new();

        let issue_data = make_issue_event("i-aaaaaa", 1);
        cache
            .apply_event(&SseEventType::IssueCreated, Some(1), &issue_data)
            .unwrap();

        let patch_data = make_patch_event("p-bbbbbb", 1);
        cache
            .apply_event(&SseEventType::PatchCreated, Some(2), &patch_data)
            .unwrap();

        let session_data = make_session_event("s-cccccc", 1);
        cache
            .apply_event(&SseEventType::SessionCreated, Some(3), &session_data)
            .unwrap();

        assert_eq!(cache.issues.len(), 1);
        assert_eq!(cache.patches.len(), 1);
        assert_eq!(cache.sessions.len(), 1);
    }
}
