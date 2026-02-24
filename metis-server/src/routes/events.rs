use crate::app::{AppState, ServerEvent, event_bus::MutationPayload};
use axum::{
    extract::{Query, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{DateTime, Utc};
use futures::channel::mpsc;
use metis_common::{
    api::v1::{
        documents::{DocumentSummaryRecord, DocumentVersionRecord},
        error::ApiError,
        events::{
            EntityEventData, EventsQuery, HeartbeatEventData, LAST_EVENT_ID_HEADER,
            ResyncEventData, SnapshotEventData, SseEventType,
        },
        issues::{IssueSummary, IssueSummaryRecord},
        jobs::JobVersionRecord,
        patches::PatchVersionRecord,
    },
    ids::{DocumentId, IssueId, PatchId, TaskId},
};
use std::{collections::HashMap, convert::Infallible, sync::Arc};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

/// GET /v1/events — Server-Sent Events stream for entity change notifications.
pub async fn get_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    let last_event_id = headers
        .get(LAST_EVENT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let filter = EventFilter::from_query(&query).map_err(ApiError::bad_request)?;

    info!(
        last_event_id = ?last_event_id,
        types_filter = ?filter.entity_types,
        "SSE events stream requested"
    );

    // Subscribe to the event bus before taking the snapshot so we don't miss
    // any events emitted between snapshot and stream start.
    let mut receiver = state.subscribe();
    let current_seq = state.event_bus().current_seq();

    let (tx, rx) = mpsc::unbounded::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        // Send initial event based on whether this is a first connect or reconnect.
        match last_event_id {
            None => {
                // First connection: send snapshot of current entity versions.
                let snapshot = build_snapshot(&state).await;
                let snapshot_event = Event::default()
                    .event(SseEventType::Snapshot.as_str())
                    .id(current_seq.to_string())
                    .json_data(&snapshot)
                    .unwrap_or_else(|_| Event::default().data("{}"));

                if tx.unbounded_send(Ok(snapshot_event)).is_err() {
                    return;
                }
            }
            Some(last_seq) => {
                // Reconnection: check if we can replay from the requested position.
                // The broadcast channel doesn't support replay, so if the client
                // reconnects, we send a resync event telling it to re-fetch state.
                if last_seq < current_seq {
                    let resync = ResyncEventData {
                        reason: "reconnected".to_string(),
                        current_seq,
                    };
                    let resync_event = Event::default()
                        .event(SseEventType::Resync.as_str())
                        .id(current_seq.to_string())
                        .json_data(&resync)
                        .unwrap_or_else(|_| Event::default().data("{}"));

                    if tx.unbounded_send(Ok(resync_event)).is_err() {
                        return;
                    }
                }
            }
        }

        // Set up heartbeat interval (15 seconds).
        let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        // Don't send a heartbeat immediately on stream start.
        heartbeat_interval.reset();

        loop {
            tokio::select! {
                result = receiver.recv() => {
                    match result {
                        Ok(event) => {
                            if !filter.matches(&event) {
                                continue;
                            }

                            let (event_type, data) = server_event_to_sse(&event, &state).await;
                            let sse_event = Event::default()
                                .event(event_type.as_str())
                                .id(event.seq().to_string())
                                .json_data(&data)
                                .unwrap_or_else(|_| Event::default().data("{}"));

                            if tx.unbounded_send(Ok(sse_event)).is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(lagged = n, "SSE client lagged behind, sending resync");
                            let current = state.event_bus().current_seq();
                            let resync = ResyncEventData {
                                reason: "lagged".to_string(),
                                current_seq: current,
                            };
                            let resync_event = Event::default()
                                .event(SseEventType::Resync.as_str())
                                .id(current.to_string())
                                .json_data(&resync)
                                .unwrap_or_else(|_| Event::default().data("{}"));

                            if tx.unbounded_send(Ok(resync_event)).is_err() {
                                break;
                            }
                            // Continue streaming from current position.
                        }
                        Err(RecvError::Closed) => {
                            break;
                        }
                    }
                }
                _ = heartbeat_interval.tick() => {
                    let heartbeat = HeartbeatEventData {
                        server_time: Utc::now(),
                    };
                    let heartbeat_event = Event::default()
                        .event(SseEventType::Heartbeat.as_str())
                        .json_data(&heartbeat)
                        .unwrap_or_else(|_| Event::default().data("{}"));

                    if tx.unbounded_send(Ok(heartbeat_event)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let sse = Sse::new(rx).keep_alive(KeepAlive::default());
    Ok(sse.into_response())
}

/// Filter for SSE events based on query parameters.
struct EventFilter {
    entity_types: Option<Vec<String>>,
    issue_ids: Option<Vec<IssueId>>,
    job_ids: Option<Vec<TaskId>>,
    patch_ids: Option<Vec<PatchId>>,
    document_ids: Option<Vec<DocumentId>>,
}

impl EventFilter {
    fn from_query(query: &EventsQuery) -> Result<Self, String> {
        let entity_types = query
            .types
            .as_ref()
            .map(|s| s.split(',').map(|t| t.trim().to_string()).collect());

        let issue_ids = query
            .issue_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<IssueId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid issue_ids: {e}"))?;

        let job_ids = query
            .job_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<TaskId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid job_ids: {e}"))?;

        let patch_ids = query
            .patch_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<PatchId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid patch_ids: {e}"))?;

        let document_ids = query
            .document_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<DocumentId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid document_ids: {e}"))?;

        Ok(Self {
            entity_types,
            issue_ids,
            job_ids,
            patch_ids,
            document_ids,
        })
    }

    fn matches(&self, event: &ServerEvent) -> bool {
        let (entity_type, entity_id) = event_entity_info(event);

        // Check entity type filter.
        if let Some(types) = &self.entity_types {
            if !types.contains(&entity_type.to_string()) {
                return false;
            }
        }

        // Check entity-specific ID filters.
        match entity_id {
            EntityId::Issue(id) => {
                if let Some(ids) = &self.issue_ids {
                    if !ids.contains(id) {
                        return false;
                    }
                }
            }
            EntityId::Task(id) => {
                if let Some(ids) = &self.job_ids {
                    if !ids.contains(id) {
                        return false;
                    }
                }
            }
            EntityId::Patch(id) => {
                if let Some(ids) = &self.patch_ids {
                    if !ids.contains(id) {
                        return false;
                    }
                }
            }
            EntityId::Document(id) => {
                if let Some(ids) = &self.document_ids {
                    if !ids.contains(id) {
                        return false;
                    }
                }
            }
        }

        true
    }
}

/// A typed entity ID extracted from a ServerEvent.
enum EntityId<'a> {
    Issue(&'a IssueId),
    Task(&'a TaskId),
    Patch(&'a PatchId),
    Document(&'a DocumentId),
}

/// Extracts the entity type category and typed entity ID from a ServerEvent.
fn event_entity_info(event: &ServerEvent) -> (&'static str, EntityId<'_>) {
    match event {
        ServerEvent::IssueCreated { issue_id, .. }
        | ServerEvent::IssueUpdated { issue_id, .. }
        | ServerEvent::IssueDeleted { issue_id, .. } => ("issues", EntityId::Issue(issue_id)),

        ServerEvent::PatchCreated { patch_id, .. }
        | ServerEvent::PatchUpdated { patch_id, .. }
        | ServerEvent::PatchDeleted { patch_id, .. } => ("patches", EntityId::Patch(patch_id)),

        ServerEvent::JobCreated { task_id, .. } | ServerEvent::JobUpdated { task_id, .. } => {
            ("jobs", EntityId::Task(task_id))
        }

        ServerEvent::DocumentCreated { document_id, .. }
        | ServerEvent::DocumentUpdated { document_id, .. }
        | ServerEvent::DocumentDeleted { document_id, .. } => {
            ("documents", EntityId::Document(document_id))
        }
    }
}

/// Serializes the `new` entity from a `MutationPayload` into a version record
/// JSON value matching the shape returned by the corresponding GET endpoint.
async fn serialize_entity(
    payload: &Arc<MutationPayload>,
    entity_id: &str,
    version: u64,
    timestamp: DateTime<Utc>,
    state: &AppState,
) -> Option<serde_json::Value> {
    let value = match payload.as_ref() {
        MutationPayload::Issue { new, .. } => {
            let api_issue: metis_common::api::v1::issues::Issue = new.clone().into();
            let summary = IssueSummary::from(&api_issue);
            let creation_time = if version == 1 {
                timestamp
            } else {
                let issue_id: IssueId = entity_id.parse().ok()?;
                state
                    .get_issue(&issue_id, true)
                    .await
                    .ok()
                    .and_then(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let record = IssueSummaryRecord::new(
                entity_id.parse().ok()?,
                version,
                timestamp,
                summary,
                Some(payload.actor().clone()),
                creation_time,
            );
            serde_json::to_value(record).ok()?
        }
        MutationPayload::Patch { new, .. } => {
            let api_patch: metis_common::api::v1::patches::Patch = new.clone().into();
            let creation_time = if version == 1 {
                timestamp
            } else {
                let patch_id: PatchId = entity_id.parse().ok()?;
                state
                    .get_patch(&patch_id, true)
                    .await
                    .ok()
                    .and_then(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let full_record = PatchVersionRecord::new(
                entity_id.parse().ok()?,
                version,
                timestamp,
                api_patch,
                Some(payload.actor().clone()),
                creation_time,
            );
            let summary_record =
                metis_common::api::v1::patches::PatchSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
        MutationPayload::Job { new, .. } => {
            let task_id: TaskId = entity_id.parse().ok()?;
            let mut api_task: metis_common::api::v1::jobs::Task = new.clone().into();
            if let Ok(log) = state.get_status_log(&task_id).await {
                api_task.creation_time = log.creation_time();
                api_task.start_time = log.start_time();
                api_task.end_time = log.end_time();
            }
            let full_record = JobVersionRecord::new(
                task_id,
                version,
                timestamp,
                api_task,
                Some(payload.actor().clone()),
            );
            let summary_record = metis_common::api::v1::jobs::JobSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
        MutationPayload::Document { new, .. } => {
            let api_doc: metis_common::api::v1::documents::Document = new.clone().into();
            let creation_time = if version == 1 {
                timestamp
            } else {
                let doc_id: DocumentId = entity_id.parse().ok()?;
                state
                    .get_document(&doc_id, true)
                    .await
                    .ok()
                    .and_then(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let full_record = DocumentVersionRecord::new(
                entity_id.parse().ok()?,
                version,
                timestamp,
                api_doc,
                Some(payload.actor().clone()),
                creation_time,
            );
            let summary_record = DocumentSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
    };
    Some(value)
}

/// Converts a ServerEvent into an SSE event type and data payload.
async fn server_event_to_sse(
    event: &ServerEvent,
    state: &AppState,
) -> (SseEventType, EntityEventData) {
    let (event_type, entity_type, entity_id, version, timestamp, payload) = match event {
        ServerEvent::IssueCreated {
            issue_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::IssueCreated,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::IssueUpdated {
            issue_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::IssueUpdated,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::IssueDeleted {
            issue_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::IssueDeleted,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::PatchCreated {
            patch_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::PatchCreated,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::PatchUpdated {
            patch_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::PatchUpdated,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::PatchDeleted {
            patch_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::PatchDeleted,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::JobCreated {
            task_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::JobCreated,
            "job",
            task_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::JobUpdated {
            task_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::JobUpdated,
            "job",
            task_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::DocumentCreated {
            document_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::DocumentCreated,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::DocumentUpdated {
            document_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::DocumentUpdated,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::DocumentDeleted {
            document_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::DocumentDeleted,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
    };

    let entity = serialize_entity(payload, &entity_id, version, timestamp, state).await;

    (
        event_type,
        EntityEventData {
            entity_type: entity_type.to_string(),
            entity_id,
            version,
            timestamp,
            entity,
        },
    )
}

/// Builds a snapshot of current entity version numbers.
async fn build_snapshot(state: &AppState) -> SnapshotEventData {
    use metis_common::api::v1::{documents, jobs, patches};

    let mut versions = HashMap::new();

    if let Ok(issues) = state.list_issues().await {
        for (id, versioned) in issues {
            versions.insert(id.to_string(), versioned.version);
        }
    }

    if let Ok(patches) = state
        .list_patches_with_query(&patches::SearchPatchesQuery::default())
        .await
    {
        for (id, versioned) in patches {
            versions.insert(id.to_string(), versioned.version);
        }
    }

    if let Ok(tasks) = state
        .list_tasks_with_query(&jobs::SearchJobsQuery::default())
        .await
    {
        for (id, versioned) in tasks {
            versions.insert(id.to_string(), versioned.version);
        }
    }

    if let Ok(documents) = state
        .list_documents(&documents::SearchDocumentsQuery::default())
        .await
    {
        for (id, versioned) in documents {
            versions.insert(id.to_string(), versioned.version);
        }
    }

    SnapshotEventData { versions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::jobs::{BundleSpec, Task};
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use crate::store::{MemoryStore, Store};
    use crate::test_utils::test_state_with_store;
    use chrono::Utc;
    use metis_common::issues::IssueId;
    use std::sync::Arc;

    fn dummy_issue() -> Issue {
        Issue::new(
            IssueType::Task,
            "sse entity test".to_string(),
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

    fn dummy_task() -> Task {
        Task::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            Some("metis-worker:latest".to_string()),
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        )
    }

    fn test_app_state() -> AppState {
        let store = Arc::new(MemoryStore::new());
        test_state_with_store(store).state
    }

    #[tokio::test]
    async fn server_event_to_sse_includes_entity_data() {
        let state = test_app_state();
        let issue_id = IssueId::new();
        let issue = dummy_issue();
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: ActorRef::test(),
        });
        let timestamp = Utc::now();
        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp,
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::IssueCreated);
        assert_eq!(data.entity_type, "issue");
        assert_eq!(data.entity_id, issue_id.to_string());
        assert_eq!(data.version, 1);

        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");

        // Verify the entity has the shape of an IssueVersionRecord.
        assert_eq!(
            obj.get("issue_id").unwrap().as_str().unwrap(),
            issue_id.to_string()
        );
        assert_eq!(obj.get("version").unwrap().as_u64().unwrap(), 1);
        assert!(obj.contains_key("timestamp"));

        // Verify the nested issue data.
        let issue_obj = obj.get("issue").expect("should contain issue field");
        assert_eq!(
            issue_obj.get("description").unwrap().as_str().unwrap(),
            "sse entity test"
        );
        assert_eq!(issue_obj.get("status").unwrap().as_str().unwrap(), "open");
    }

    #[tokio::test]
    async fn server_event_to_sse_includes_entity_on_update() {
        let state = test_app_state();
        let issue_id = IssueId::new();
        let old_issue = dummy_issue();
        let mut new_issue = old_issue.clone();
        new_issue.status = IssueStatus::InProgress;
        new_issue.description = "updated description".to_string();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::IssueUpdated);
        let entity = data
            .entity
            .expect("entity should be present for update events");
        let issue_obj = entity.get("issue").expect("should contain issue field");
        assert_eq!(
            issue_obj.get("description").unwrap().as_str().unwrap(),
            "updated description"
        );
        assert_eq!(
            issue_obj.get("status").unwrap().as_str().unwrap(),
            "in-progress"
        );
    }

    #[tokio::test]
    async fn server_event_to_sse_includes_entity_on_delete() {
        let state = test_app_state();
        let issue_id = IssueId::new();
        let old_issue = dummy_issue();
        let mut deleted_issue = old_issue.clone();
        deleted_issue.deleted = true;

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: deleted_issue,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueDeleted {
            seq: 3,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let (_, data) = server_event_to_sse(&event, &state).await;

        let entity = data
            .entity
            .expect("entity should be present for delete events");
        let issue_obj = entity.get("issue").expect("should contain issue field");
        assert!(issue_obj.get("deleted").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn server_event_to_sse_job_includes_time_fields() {
        let store = Arc::new(MemoryStore::new());
        let handles = test_state_with_store(store.clone());
        let state = handles.state;

        // Create a task in the store so the status log exists.
        let task = dummy_task();
        let (task_id, _) = store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Transition task to running so start_time is populated.
        state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&task_id, ActorRef::test())
            .await
            .unwrap();

        // Build a JobUpdated event (simulating what the event bus emits).
        let mut running_task = task.clone();
        running_task.status = Status::Running;
        let payload = Arc::new(MutationPayload::Job {
            old: Some(task),
            new: running_task,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::JobUpdated {
            seq: 3,
            task_id: task_id.clone(),
            version: 3,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::JobUpdated);
        assert_eq!(data.entity_type, "job");
        assert_eq!(data.entity_id, task_id.to_string());

        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");
        let task_obj = obj.get("task").expect("should contain task field");

        // Verify time fields are populated.
        assert!(
            task_obj.get("creation_time").unwrap().as_str().is_some(),
            "creation_time should be non-null"
        );
        assert!(
            task_obj.get("start_time").unwrap().as_str().is_some(),
            "start_time should be non-null for a running job"
        );
    }

    #[tokio::test]
    async fn server_event_to_sse_job_created_includes_creation_time() {
        let store = Arc::new(MemoryStore::new());
        let handles = test_state_with_store(store.clone());
        let state = handles.state;

        // Create a task in the store so the status log exists.
        let task = dummy_task();
        let (task_id, _) = store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Build a JobCreated event.
        let payload = Arc::new(MutationPayload::Job {
            old: None,
            new: task,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::JobCreated {
            seq: 1,
            task_id: task_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::JobCreated);
        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");
        let task_obj = obj.get("task").expect("should contain task field");

        // creation_time should be present for a newly created job.
        assert!(
            task_obj.get("creation_time").unwrap().as_str().is_some(),
            "creation_time should be non-null"
        );
        // start_time should be absent since the job hasn't started
        // (the field is skipped when None due to skip_serializing_if).
        assert!(
            task_obj.get("start_time").is_none(),
            "start_time should be absent for a created (not yet running) job"
        );
    }
}
