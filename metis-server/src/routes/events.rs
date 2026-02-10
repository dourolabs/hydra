use crate::app::{AppState, ServerEvent};
use axum::{
    extract::{Query, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::Utc;
use futures::channel::mpsc;
use metis_common::{
    api::v1::{
        error::ApiError,
        events::{
            EntityEventData, EventsQuery, HeartbeatEventData, LAST_EVENT_ID_HEADER,
            ResyncEventData, SnapshotEventData, SseEventType,
        },
    },
    ids::{DocumentId, IssueId, PatchId, TaskId},
};
use std::{collections::HashMap, convert::Infallible};
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

                            let (event_type, data) = server_event_to_sse(&event);
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

/// Converts a ServerEvent into an SSE event type and data payload.
fn server_event_to_sse(event: &ServerEvent) -> (SseEventType, EntityEventData) {
    let (event_type, entity_type, entity_id, version, timestamp) = match event {
        ServerEvent::IssueCreated {
            issue_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::IssueCreated,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::IssueUpdated {
            issue_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::IssueUpdated,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::IssueDeleted {
            issue_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::IssueDeleted,
            "issue",
            issue_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::PatchCreated {
            patch_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::PatchCreated,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::PatchUpdated {
            patch_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::PatchUpdated,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::PatchDeleted {
            patch_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::PatchDeleted,
            "patch",
            patch_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::JobCreated {
            task_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::JobCreated,
            "job",
            task_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::JobUpdated {
            task_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::JobUpdated,
            "job",
            task_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::DocumentCreated {
            document_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::DocumentCreated,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::DocumentUpdated {
            document_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::DocumentUpdated,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
        ),
        ServerEvent::DocumentDeleted {
            document_id,
            version,
            timestamp,
            ..
        } => (
            SseEventType::DocumentDeleted,
            "document",
            document_id.to_string(),
            *version,
            *timestamp,
        ),
    };

    (
        event_type,
        EntityEventData {
            entity_type: entity_type.to_string(),
            entity_id,
            version,
            timestamp,
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
