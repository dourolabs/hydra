use crate::app::{AppState, ServerEvent, event_bus::EntityId, event_bus::MutationPayload};
use crate::domain::actors::Actor;
use crate::job_engine::JobStatus;
use axum::{
    Extension,
    extract::{Query, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{DateTime, Utc};
use futures::{StreamExt, channel::mpsc};
use hydra_common::{
    LabelId,
    api::v1::{
        documents::{DocumentSummaryRecord, DocumentVersionRecord},
        error::ApiError,
        events::{
            ConnectedEventData, EntityEventData, EventsQuery, HeartbeatEventData,
            LAST_EVENT_ID_HEADER, ResyncEventData, SessionLogEventData, SseEventType,
        },
        issues::IssueSummaryRecord,
        patches::PatchVersionRecord,
        sessions::SessionVersionRecord,
    },
    ids::{DocumentId, IssueId, PatchId, SessionId},
};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

/// GET /v1/events — Server-Sent Events stream for entity change notifications.
pub async fn get_events(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
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
        actor = %actor.name(),
        "SSE events stream requested"
    );

    // Subscribe to the event bus before taking the snapshot so we don't miss
    // any events emitted between snapshot and stream start.
    let mut receiver = state.subscribe();
    let current_seq = state.event_bus().current_seq();

    let (tx, rx) = mpsc::unbounded::<Result<Event, Infallible>>();

    // When the caller subscribes to one or more `session_ids`, multiplex each
    // subscribed session's log stream into this SSE stream so the browser only
    // needs a single EventSource per tab (rather than one per visible session).
    // Each spawned forwarder shares the same `tx`; when the SSE response is
    // dropped the channel closes and the forwarders exit on their next send.
    if let Some(session_ids) = filter.session_ids.clone() {
        for session_id in session_ids {
            spawn_session_log_forwarder(state.clone(), session_id, tx.clone());
        }
    }

    tokio::spawn(async move {
        // Send initial event based on whether this is a first connect or reconnect.
        match last_event_id {
            None => {
                // First connection: send a lightweight connected event with
                // the current sequence number for reconnection support.
                let connected = ConnectedEventData { current_seq };
                let connected_event = Event::default()
                    .event(SseEventType::Connected.as_str())
                    .id(current_seq.to_string())
                    .json_data(&connected)
                    .unwrap_or_else(|_| Event::default().data("{}"));

                if tx.unbounded_send(Ok(connected_event)).is_err() {
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

                            let sse_event = build_sse_event(&event, &state).await;

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

    info!(actor = %actor.name(), "get_events stream established");
    let sse = Sse::new(rx).keep_alive(KeepAlive::default());
    Ok(sse.into_response())
}

/// Filter for SSE events based on query parameters.
struct EventFilter {
    entity_types: Option<Vec<String>>,
    issue_ids: Option<Vec<IssueId>>,
    session_ids: Option<Vec<SessionId>>,
    patch_ids: Option<Vec<PatchId>>,
    label_ids: Option<Vec<LabelId>>,
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

        let session_ids = query
            .session_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<SessionId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid session_ids: {e}"))?;

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

        let label_ids = query
            .label_ids
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<LabelId>())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|e| format!("invalid label_ids: {e}"))?;

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
            session_ids,
            patch_ids,
            label_ids,
            document_ids,
        })
    }

    fn matches(&self, event: &ServerEvent) -> bool {
        let (entity_type, entity_id) = event.entity_info();

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
            EntityId::Session(id) => {
                if let Some(ids) = &self.session_ids {
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
            EntityId::Label(id) => {
                if let Some(ids) = &self.label_ids {
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
            EntityId::Conversation(_) => {
                // No ID filter for conversations yet
            }
        }

        true
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
            let summary = crate::routes::issue_response::build_issue_summary_response(state, new)
                .await
                .ok()?;
            let creation_time = if version == 1 {
                timestamp
            } else {
                let issue_id: IssueId = entity_id.parse().ok()?;
                state
                    .get_issue(&issue_id, true)
                    .await
                    .ok()
                    .map(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let issue_id: IssueId = entity_id.parse().ok()?;
            let labels = state
                .get_labels_for_object(&hydra_common::HydraId::from(issue_id.clone()))
                .await
                .unwrap_or_default();
            let record = IssueSummaryRecord::new(
                issue_id,
                version,
                timestamp,
                summary,
                Some(payload.actor().clone()),
                creation_time,
                labels,
            );
            serde_json::to_value(record).ok()?
        }
        MutationPayload::Patch { new, .. } => {
            let api_patch: hydra_common::api::v1::patches::Patch = new.clone().into();
            let patch_id: PatchId = entity_id.parse().ok()?;
            let creation_time = if version == 1 {
                timestamp
            } else {
                state
                    .get_patch(&patch_id, true)
                    .await
                    .ok()
                    .map(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let labels = state
                .get_labels_for_object(&hydra_common::HydraId::from(patch_id.clone()))
                .await
                .unwrap_or_default();
            let full_record = PatchVersionRecord::new(
                patch_id,
                version,
                timestamp,
                api_patch,
                Some(payload.actor().clone()),
                creation_time,
                labels,
            );
            let summary_record =
                hydra_common::api::v1::patches::PatchSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
        MutationPayload::Session { new, .. } => {
            let task_id: SessionId = entity_id.parse().ok()?;
            let mut api_task: hydra_common::api::v1::sessions::Session = new.clone().into();
            if let Ok(log) = state.get_status_log(&task_id).await {
                api_task.creation_time = log.creation_time();
                api_task.start_time = log.start_time();
                api_task.end_time = log.end_time();
            }
            let full_record = SessionVersionRecord::new(
                task_id,
                version,
                timestamp,
                api_task,
                Some(payload.actor().clone()),
            );
            let summary_record =
                hydra_common::api::v1::sessions::SessionSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
        MutationPayload::Document { new, .. } => {
            let api_doc: hydra_common::api::v1::documents::Document = new.clone().into();
            let doc_id: DocumentId = entity_id.parse().ok()?;
            let creation_time = if version == 1 {
                timestamp
            } else {
                state
                    .get_document(&doc_id, true)
                    .await
                    .ok()
                    .map(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let labels = state
                .get_labels_for_object(&hydra_common::HydraId::from(doc_id.clone()))
                .await
                .unwrap_or_default();
            let full_record = DocumentVersionRecord::new(
                doc_id,
                version,
                timestamp,
                api_doc,
                Some(payload.actor().clone()),
                creation_time,
                labels,
            );
            let summary_record = DocumentSummaryRecord::from(&full_record);
            serde_json::to_value(summary_record).ok()?
        }
        MutationPayload::Label { new, .. } => {
            let label_id: LabelId = entity_id.parse().ok()?;
            let record = hydra_common::api::v1::labels::LabelRecord::new(
                label_id,
                new.name.clone(),
                new.color.clone(),
                new.recurse,
                new.hidden,
                new.created_at,
                new.updated_at,
            );
            serde_json::to_value(record).ok()?
        }
        MutationPayload::Conversation { new, .. } => {
            let conversation_id: hydra_common::ConversationId = entity_id.parse().ok()?;
            let creation_time = if version == 1 {
                timestamp
            } else {
                state
                    .store()
                    .get_conversation(&conversation_id, true)
                    .await
                    .ok()
                    .map(|v| v.creation_time)
                    .unwrap_or(timestamp)
            };
            let api_conv = new.to_api(conversation_id, creation_time, timestamp);
            serde_json::to_value(api_conv).ok()?
        }
        MutationPayload::SessionEvent { event, .. } => {
            let api_event: hydra_common::api::v1::sessions::SessionEvent = event.clone().into();
            serde_json::to_value(api_event).ok()?
        }
        MutationPayload::SessionState { .. } => {
            // The state blob itself is fetched by subscribers via
            // `get_session_state`; the SSE notification carries no payload.
            return None;
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
        ServerEvent::SessionCreated {
            session_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::SessionCreated,
            "session",
            session_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::SessionUpdated {
            session_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::SessionUpdated,
            "session",
            session_id.to_string(),
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
        ServerEvent::LabelCreated {
            label_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::LabelCreated,
            "label",
            label_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::LabelUpdated {
            label_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::LabelUpdated,
            "label",
            label_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::LabelDeleted {
            label_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::LabelDeleted,
            "label",
            label_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::ConversationCreated {
            conversation_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::ConversationCreated,
            "conversation",
            conversation_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::ConversationUpdated {
            conversation_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::ConversationUpdated,
            "conversation",
            conversation_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::SessionEventCreated {
            session_id,
            version,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::SessionEventCreated,
            "session_event",
            session_id.to_string(),
            *version,
            *timestamp,
            payload,
        ),
        ServerEvent::SessionStateUpdated {
            session_id,
            timestamp,
            payload,
            ..
        } => (
            SseEventType::SessionStateUpdated,
            "session_state",
            session_id.to_string(),
            0,
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

/// Builds an SSE `Event` from a `ServerEvent`. All entity types (including
/// notifications) go through the standard `server_event_to_sse` path.
async fn build_sse_event(event: &ServerEvent, state: &AppState) -> Event {
    let (event_type, data) = server_event_to_sse(event, state).await;
    Event::default()
        .event(event_type.as_str())
        .id(event.seq().to_string())
        .json_data(&data)
        .unwrap_or_else(|_| Event::default().data("{}"))
}

/// Spawns a background task that streams log chunks for `session_id` and
/// forwards them as `session_log` SSE events on the shared `tx` channel. The
/// task exits when the session has no live log stream, when the log source
/// closes, or when the SSE client disconnects (detected via `tx` send error).
fn spawn_session_log_forwarder(
    state: AppState,
    session_id: SessionId,
    tx: mpsc::UnboundedSender<Result<Event, Infallible>>,
) {
    tokio::spawn(async move {
        let job = match state.job_engine.find_job_by_hydra_id(&session_id).await {
            Ok(job) => job,
            Err(err) => {
                info!(
                    session_id = %session_id,
                    error = ?err,
                    "session_log: no job found for subscribed session; skipping log forwarding"
                );
                return;
            }
        };

        let follow = job.status == JobStatus::Running;
        let mut receiver = match state.job_engine.get_logs_stream(&session_id, follow) {
            Ok(r) => r,
            Err(err) => {
                warn!(
                    session_id = %session_id,
                    error = ?err,
                    "session_log: failed to open log stream"
                );
                return;
            }
        };

        let session_id_str = session_id.to_string();
        while let Some(chunk) = receiver.next().await {
            let payload = SessionLogEventData {
                session_id: session_id_str.clone(),
                chunk,
            };
            let event = Event::default()
                .event(SseEventType::SessionLog.as_str())
                .json_data(&payload)
                .unwrap_or_else(|_| Event::default().data("{}"));
            if tx.unbounded_send(Ok(event)).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::sessions::Session;
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use crate::store::{MemoryStore, Store};
    use crate::test_utils::test_state_with_store;
    use chrono::Utc;
    use hydra_common::issues::IssueId;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn dummy_issue() -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "sse entity test".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open.into(),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn dummy_task() -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        use crate::routes::sessions::mount_spec_from_create_request;
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            Some("hydra-worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
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
        assert_eq!(
            issue_obj
                .get("status")
                .unwrap()
                .get("key")
                .unwrap()
                .as_str()
                .unwrap(),
            "open"
        );
    }

    #[tokio::test]
    async fn server_event_to_sse_includes_entity_on_update() {
        let state = test_app_state();
        let issue_id = IssueId::new();
        let old_issue = dummy_issue();
        let mut new_issue = old_issue.clone();
        new_issue.status = IssueStatus::InProgress.into();
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
            issue_obj
                .get("status")
                .unwrap()
                .get("key")
                .unwrap()
                .as_str()
                .unwrap(),
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
            .add_session(task.clone(), Utc::now(), &ActorRef::test())
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

        // Build a SessionUpdated event (simulating what the event bus emits).
        let mut running_task = task.clone();
        running_task.status = Status::Running;
        let payload = Arc::new(MutationPayload::Session {
            old: Some(task),
            new: running_task,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::SessionUpdated {
            seq: 3,
            session_id: task_id.clone(),
            version: 3,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::SessionUpdated);
        assert_eq!(data.entity_type, "session");
        assert_eq!(data.entity_id, task_id.to_string());

        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");
        let task_obj = obj.get("session").expect("should contain session field");

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
            .add_session(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Build a SessionCreated event.
        let payload = Arc::new(MutationPayload::Session {
            old: None,
            new: task,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::SessionCreated {
            seq: 1,
            session_id: task_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;

        assert_eq!(event_type, SseEventType::SessionCreated);
        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");
        let task_obj = obj.get("session").expect("should contain session field");

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

    #[tokio::test]
    async fn serialize_entity_includes_labels_for_issue() {
        let store = Arc::new(MemoryStore::new());
        let handles = test_state_with_store(store.clone());
        let state = handles.state;

        // Create an issue in the store so the labels lookup works.
        let issue = dummy_issue();
        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Add a label and associate it with the issue.
        let label = crate::domain::labels::Label::new(
            "bug".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = store.add_label(label).await.unwrap();
        let object_id = hydra_common::HydraId::from(issue_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        // Build an IssueUpdated event.
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let (_, data) = server_event_to_sse(&event, &state).await;

        let entity = data.entity.expect("entity should be present");
        let issue_obj = entity.get("issue").expect("should contain issue field");
        let labels = issue_obj
            .get("labels")
            .expect("should contain labels field")
            .as_array()
            .expect("labels should be an array");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].get("name").unwrap().as_str().unwrap(), "bug");
    }

    #[tokio::test]
    async fn serialize_entity_includes_labels_for_patch() {
        use crate::domain::patches::{Patch, PatchStatus};

        let store = Arc::new(MemoryStore::new());
        let handles = test_state_with_store(store.clone());
        let state = handles.state;

        let patch = Patch::new(
            "Test patch".to_string(),
            "description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("creator"),
            Vec::new(),
            "test/repo".parse().unwrap(),
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let label = crate::domain::labels::Label::new(
            "urgent".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
        );
        let label_id = store.add_label(label).await.unwrap();
        let object_id = hydra_common::HydraId::from(patch_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(patch.clone()),
            new: patch,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::PatchUpdated {
            seq: 2,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let (_, data) = server_event_to_sse(&event, &state).await;

        let entity = data.entity.expect("entity should be present");
        let patch_obj = entity.get("patch").expect("should contain patch field");
        let labels = patch_obj
            .get("labels")
            .expect("should contain labels field")
            .as_array()
            .expect("labels should be an array");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].get("name").unwrap().as_str().unwrap(), "urgent");
    }

    #[tokio::test]
    async fn serialize_entity_includes_labels_for_document() {
        use crate::domain::documents::Document;

        let store = Arc::new(MemoryStore::new());
        let handles = test_state_with_store(store.clone());
        let state = handles.state;

        let doc = Document {
            title: "Test doc".to_string(),
            body_markdown: "content".to_string(),
            path: None,
            deleted: false,
        };
        let (doc_id, _) = store
            .add_document(doc.clone(), &ActorRef::test())
            .await
            .unwrap();

        let label = crate::domain::labels::Label::new(
            "docs".to_string(),
            "#3498db".parse().unwrap(),
            true,
            false,
        );
        let label_id = store.add_label(label).await.unwrap();
        let object_id = hydra_common::HydraId::from(doc_id.clone());
        store
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Document {
            old: Some(doc.clone()),
            new: doc,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::DocumentUpdated {
            seq: 2,
            document_id: doc_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let (_, data) = server_event_to_sse(&event, &state).await;

        let entity = data.entity.expect("entity should be present");
        let doc_obj = entity
            .get("document")
            .expect("should contain document field");
        let labels = doc_obj
            .get("labels")
            .expect("should contain labels field")
            .as_array()
            .expect("labels should be an array");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].get("name").unwrap().as_str().unwrap(), "docs");
    }

    #[tokio::test]
    async fn server_event_to_sse_session_event_created() {
        use crate::domain::sessions::SessionEvent;
        use hydra_common::SessionId;

        let state = test_app_state();
        let session_id = SessionId::new();
        let session_event = SessionEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        let payload = Arc::new(MutationPayload::SessionEvent {
            session_id: session_id.clone(),
            event: session_event,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::SessionEventCreated {
            seq: 7,
            session_id: session_id.clone(),
            version: 3,
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;
        assert_eq!(event_type, SseEventType::SessionEventCreated);
        assert_eq!(data.entity_type, "session_event");
        assert_eq!(data.entity_id, session_id.to_string());
        assert_eq!(data.version, 3);

        // The session-event payload is serialized as the API SessionEvent JSON.
        let entity = data.entity.expect("entity should be present");
        let obj = entity.as_object().expect("entity should be a JSON object");
        assert_eq!(obj.get("type").unwrap().as_str().unwrap(), "user_message");
        assert_eq!(obj.get("content").unwrap().as_str().unwrap(), "hello");
    }

    #[tokio::test]
    async fn server_event_to_sse_session_state_updated() {
        use hydra_common::SessionId;

        let state = test_app_state();
        let session_id = SessionId::new();
        let payload = Arc::new(MutationPayload::SessionState {
            session_id: session_id.clone(),
            actor: ActorRef::test(),
        });
        let event = ServerEvent::SessionStateUpdated {
            seq: 11,
            session_id: session_id.clone(),
            timestamp: Utc::now(),
            payload,
        };

        let (event_type, data) = server_event_to_sse(&event, &state).await;
        assert_eq!(event_type, SseEventType::SessionStateUpdated);
        assert_eq!(data.entity_type, "session_state");
        assert_eq!(data.entity_id, session_id.to_string());
        // No body — consumers must fetch the state blob via `get_session_state`.
        assert!(data.entity.is_none());
    }

    #[test]
    fn event_filter_sessions_matches_session_event_created() {
        use hydra_common::SessionId;

        let session_id = SessionId::new();
        let payload = Arc::new(MutationPayload::SessionEvent {
            session_id: session_id.clone(),
            event: crate::domain::sessions::SessionEvent::Closed {
                timestamp: Utc::now(),
            },
            actor: ActorRef::test(),
        });
        let event = ServerEvent::SessionEventCreated {
            seq: 1,
            session_id: session_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Default filter (no types specified) matches.
        let default_filter = EventFilter::from_query(&EventsQuery::default()).unwrap();
        assert!(default_filter.matches(&event));

        // types=sessions matches.
        let sessions_query = EventsQuery {
            types: Some("sessions".to_string()),
            ..Default::default()
        };
        let sessions_filter = EventFilter::from_query(&sessions_query).unwrap();
        assert!(sessions_filter.matches(&event));

        // types=issues does NOT match.
        let issues_query = EventsQuery {
            types: Some("issues".to_string()),
            ..Default::default()
        };
        let issues_filter = EventFilter::from_query(&issues_query).unwrap();
        assert!(!issues_filter.matches(&event));

        // session_ids filter targets a different session — no match.
        let other_id = SessionId::new();
        let other_query = EventsQuery {
            session_ids: Some(other_id.to_string()),
            ..Default::default()
        };
        let other_filter = EventFilter::from_query(&other_query).unwrap();
        assert!(!other_filter.matches(&event));

        // session_ids filter matches our id.
        let matching_query = EventsQuery {
            session_ids: Some(session_id.to_string()),
            ..Default::default()
        };
        let matching_filter = EventFilter::from_query(&matching_query).unwrap();
        assert!(matching_filter.matches(&event));
    }
}
