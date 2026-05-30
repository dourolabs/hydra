use super::agents::AgentName;
use super::issues::SessionSettings;
use super::sessions::SessionEvent;
use crate::{ConversationId, SessionId, users::Username};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchConversationsQuery {
    /// Free-text search across conversation title, agent name, and ID.
    #[serde(default)]
    pub q: Option<String>,
    /// Filter by conversation status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ConversationStatus>,
    /// Filter by creator username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// Include soft-deleted conversations in results.
    #[serde(default)]
    pub include_deleted: Option<bool>,
    /// Maximum number of results to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Active,
    Idle,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Conversation {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<AgentName>,
    pub status: ConversationStatus,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<AgentName>,
        status: ConversationStatus,
        creator: Username,
        session_settings: SessionSettings,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            conversation_id,
            title,
            agent_name,
            status,
            creator,
            session_settings,
            created_at,
            updated_at,
        }
    }
}

impl crate::graph::GraphView for Conversation {
    const KIND: crate::graph::ObjectKind = crate::graph::ObjectKind::Conversation;

    fn view_l1(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "status": self.status,
        })
    }

    fn view_l2(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "status": self.status,
            "agent_name": self.agent_name,
            "creator": self.creator,
            "updated_at": self.updated_at,
        })
    }

    fn view_l3(&self) -> Value {
        serde_json::to_value(self).expect("Conversation serialization is infallible")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ConversationSummary {
    pub conversation_id: ConversationId,
    pub title: Option<String>,
    pub agent_name: Option<AgentName>,
    pub status: ConversationStatus,
    pub event_count: usize,
    pub last_event_preview: Option<String>,
    pub creator: Username,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConversationSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: ConversationId,
        title: Option<String>,
        agent_name: Option<AgentName>,
        status: ConversationStatus,
        event_count: usize,
        last_event_preview: Option<String>,
        creator: Username,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            conversation_id,
            title,
            agent_name,
            status,
            event_count,
            last_event_preview,
            creator,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CreateConversationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<AgentName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_settings: Option<SessionSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UpdateConversationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SendMessageRequest {
    pub content: String,
}

/// Messages sent from the worker to the server over the relay WebSocket.
///
/// The first message must be either `Fresh` or `Reconnecting`; the server
/// bails the connection on any other first-inbound variant. Phase 2's
/// `Ready` signals that the worker has finished context negotiation and
/// is awaiting `FirstMessage`. Phase 3 carries session events,
/// session-state uploads, and (on graceful shutdown) `EndSessionAck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// Phase 1 — fresh worker boot; expect `ResumeContext`.
    Fresh,
    /// Phase 1 — worker reconnecting after a transient drop; expect `CatchUp`.
    ///
    /// `last_received_session_event_index` is `None` when the worker has not
    /// yet received any forwarded `ServerMessage::Event` (the server returns
    /// the full log in that case); `Some(N)` requests events with `event_index
    /// > N` only.
    Reconnecting {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_received_session_event_index: Option<usize>,
    },
    /// Phase 1 — native resume materialization failed (or blob was absent
    /// with a prior session id), so the worker asks for the prior
    /// session's transcript to use as primer text.
    RequestTranscript { prior_session_id: SessionId },
    /// Phase 2 — worker has finished context negotiation and is awaiting
    /// `FirstMessage`.
    Ready,
    /// Phase 3 — a session event (assistant message, tool use, etc.).
    Event { event: SessionEvent },
    /// Phase 3 (anytime) — a session state upload for resumption support.
    SessionStateUpload {
        #[cfg_attr(feature = "ts", ts(type = "number[]"))]
        data: Vec<u8>,
    },
    /// Phase 3 — acknowledgment that the worker has observed
    /// `ServerMessage::EndSession` and is closing the session. Sent
    /// immediately before the WS close, after the final
    /// `SessionStateUpload` and `Closed` event.
    EndSessionAck,
}

/// Payload carried inside `WorkerMessage::SessionStateUpload { data }` (as JSON
/// bytes) so a resumed worker can restore Claude's transcript file on disk and
/// invoke `claude --resume <session_id>` against the same conversation.
///
/// The wire envelope (`SessionStateUpload`) is unchanged; the structured
/// payload lives inside the `data` bytes. The enum is `#[serde(tag = "version")]`
/// so future revisions can be added without breaking older workers — an
/// unknown variant deserialization fails fast and the resumer falls back to
/// the context-primer path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "version", rename_all = "snake_case")]
pub enum SessionStatePayload {
    /// Version 1: Claude's session UUID plus an optional transcript blob.
    V1 {
        /// Claude's internal session UUID, extracted from the JSONL stream
        /// the prior worker observed on Claude's stdout. Used as the
        /// argument to `claude --resume <session_id>`.
        session_id: String,
        /// The bytes of Claude's per-project transcript file at the moment
        /// of upload. `None` means the worker captured a `session_id` but
        /// could not read the transcript file (e.g. missing or unreadable);
        /// in that case the resumer should fall back to the primer path.
        #[cfg_attr(feature = "ts", ts(type = "number[] | null"))]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript: Option<Vec<u8>>,
    },
}

/// Messages sent from the server to the worker over the relay WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Phase 1 (Fresh) — server-side resume context. `resume_blob` carries
    /// the persisted opaque bytes (if any); `prior_session_id` is set when
    /// the session resumes from another. Both are `None` for a brand-new
    /// session with no lineage.
    ResumeContext {
        #[cfg_attr(feature = "ts", ts(type = "number[] | null"))]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_blob: Option<Vec<u8>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prior_session_id: Option<SessionId>,
    },
    /// Phase 1 (RequestTranscript fallback) — the prior session's event log
    /// for the worker to use as primer text.
    Transcript { events: Vec<SessionEvent> },
    /// Phase 1 (Reconnecting) — events past the worker's last seen index,
    /// each tagged with its per-session `event_index` so the worker can
    /// resume tracking the running max post-catch-up.
    CatchUp { events: Vec<CatchUpEvent> },
    /// Phase 2 — the first prompt + user message, combined into a single
    /// turn. Either string may be empty; the worker concatenates them with
    /// a `\n\n` separator (collapsing when one side is empty).
    FirstMessage {
        agent_prompt: String,
        user_message: String,
    },
    /// Phase 3 — a session event forwarded to the worker (e.g., a user
    /// message in interactive mode). `event_index` is the per-session
    /// `VersionNumber` assigned by the server's session-event log so the
    /// worker can track the running max for a future `Reconnecting` opener.
    Event {
        event: SessionEvent,
        event_index: usize,
    },
    /// Phase 3 — server requests graceful shutdown of the worker session.
    /// The worker signals graceful exit to the model wrapper (interactive:
    /// stdin EOF), awaits the model's natural exit, runs the unified
    /// cleanup-and-close sequence (`SessionStateUpload` → `Closed` event →
    /// `EndSessionAck`), and closes the WS.
    EndSession,
}

/// A `SessionEvent` together with its per-session `event_index`. Used as
/// the payload of `ServerMessage::CatchUp` so the worker can update its
/// running max from the catch-up slice itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CatchUpEvent {
    pub event: SessionEvent,
    pub event_index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn conversation_status_round_trip() {
        for status in [
            ConversationStatus::Active,
            ConversationStatus::Idle,
            ConversationStatus::Closed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: ConversationStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn worker_message_fresh_round_trip() {
        let msg = WorkerMessage::Fresh;
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"fresh""#));
    }

    #[test]
    fn worker_message_reconnecting_some_round_trip() {
        let msg = WorkerMessage::Reconnecting {
            last_received_session_event_index: Some(10),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"reconnecting""#));
        assert!(json.contains(r#""last_received_session_event_index":10"#));
    }

    #[test]
    fn worker_message_reconnecting_none_round_trip() {
        let msg = WorkerMessage::Reconnecting {
            last_received_session_event_index: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"reconnecting""#));
        assert!(
            !json.contains("last_received_session_event_index"),
            "None variant must omit the field on the wire, got {json}"
        );
    }

    #[test]
    fn worker_message_request_transcript_round_trip() {
        let msg = WorkerMessage::RequestTranscript {
            prior_session_id: SessionId::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"request_transcript""#));
    }

    #[test]
    fn worker_message_ready_round_trip() {
        let msg = WorkerMessage::Ready;
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"ready""#));
    }

    #[test]
    fn server_message_resume_context_round_trip() {
        let msg = ServerMessage::ResumeContext {
            resume_blob: Some(vec![1, 2, 3, 4]),
            prior_session_id: Some(SessionId::new()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"resume_context""#));
    }

    #[test]
    fn server_message_transcript_round_trip() {
        let msg = ServerMessage::Transcript {
            events: vec![SessionEvent::UserMessage {
                content: "primer".to_string(),
                timestamp: Utc::now(),
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"transcript""#));
    }

    #[test]
    fn server_message_first_message_round_trip() {
        let msg = ServerMessage::FirstMessage {
            agent_prompt: "you are a helpful assistant".to_string(),
            user_message: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"first_message""#));
    }

    #[test]
    fn create_conversation_request_without_agent_name() {
        let json = r#"{"message":"Hello"}"#;
        let req: CreateConversationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message.as_deref(), Some("Hello"));
        assert_eq!(req.agent_name, None);
        assert_eq!(req.session_settings, None);
    }

    #[test]
    fn create_conversation_request_with_session_settings_round_trip() {
        let req = CreateConversationRequest {
            message: Some("Hello".to_string()),
            agent_name: Some(AgentName::try_new("my-agent").unwrap()),
            session_settings: Some(SessionSettings {
                repo_name: Some(crate::RepoName::from_str("org/repo").unwrap()),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
        assert!(json.contains("session_settings"));
        assert!(json.contains("org/repo"));
    }

    #[test]
    fn create_conversation_request_without_session_settings_omits_field() {
        let req = CreateConversationRequest {
            message: Some("Hello".to_string()),
            agent_name: None,
            session_settings: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("session_settings"));
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn create_conversation_request_without_message_round_trip() {
        let req = CreateConversationRequest {
            message: None,
            agent_name: None,
            session_settings: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("message"));
        let deserialized: CreateConversationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn create_conversation_request_deserializes_empty_object() {
        let req: CreateConversationRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.message, None);
        assert_eq!(req.agent_name, None);
        assert_eq!(req.session_settings, None);
    }

    #[test]
    fn worker_message_event_round_trip() {
        let msg = WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
                content: "Hello!".to_string(),
                timestamp: Utc::now(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"event""#));
    }

    #[test]
    fn server_message_event_carries_event_index() {
        let msg = ServerMessage::Event {
            event: SessionEvent::UserMessage {
                content: "hi".to_string(),
                timestamp: Utc::now(),
            },
            event_index: 7,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""event_index":7"#));
    }

    #[test]
    fn server_message_catch_up_carries_per_item_event_index() {
        let msg = ServerMessage::CatchUp {
            events: vec![
                CatchUpEvent {
                    event: SessionEvent::UserMessage {
                        content: "a".to_string(),
                        timestamp: Utc::now(),
                    },
                    event_index: 3,
                },
                CatchUpEvent {
                    event: SessionEvent::AssistantMessage {
                        content: "b".to_string(),
                        timestamp: Utc::now(),
                    },
                    event_index: 4,
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""event_index":3"#));
        assert!(json.contains(r#""event_index":4"#));
    }

    #[test]
    fn worker_message_end_session_ack_round_trip() {
        let msg = WorkerMessage::EndSessionAck;
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"end_session_ack""#));
    }

    #[test]
    fn server_message_end_session_round_trip() {
        let msg = ServerMessage::EndSession;
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"end_session""#));
    }

    #[test]
    fn worker_message_session_state_upload_round_trip() {
        let msg = WorkerMessage::SessionStateUpload {
            data: vec![10, 20, 30],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WorkerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"session_state_upload""#));
    }

    #[test]
    fn server_message_catch_up_round_trip() {
        let msg = ServerMessage::CatchUp {
            events: vec![CatchUpEvent {
                event: SessionEvent::UserMessage {
                    content: "hi".to_string(),
                    timestamp: Utc::now(),
                },
                event_index: 1,
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
        assert!(json.contains(r#""type":"catch_up""#));
    }

    #[test]
    fn session_state_payload_v1_round_trip_with_transcript() {
        let payload = SessionStatePayload::V1 {
            session_id: "abc-123".to_string(),
            transcript: Some(b"{\"type\":\"summary\"}\n".to_vec()),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let parsed: SessionStatePayload = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload, parsed);
        let as_str = std::str::from_utf8(&bytes).unwrap();
        assert!(
            as_str.contains("\"version\":\"v1\""),
            "tagged serialization expected, got {as_str}"
        );
    }

    #[test]
    fn session_state_payload_v1_round_trip_without_transcript() {
        let payload = SessionStatePayload::V1 {
            session_id: "abc-123".to_string(),
            transcript: None,
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let as_str = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !as_str.contains("transcript"),
            "missing transcript should be omitted, got {as_str}"
        );
        let parsed: SessionStatePayload = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload, parsed);
    }

    #[test]
    fn session_state_payload_unknown_version_rejected() {
        // An unknown future version should not silently deserialize — the
        // resumer must observe the parse failure and fall back to the
        // primer path.
        let bytes = br#"{"version":"v999","session_id":"x"}"#;
        let parsed: Result<SessionStatePayload, _> = serde_json::from_slice(bytes);
        assert!(parsed.is_err(), "unknown versions must fail to parse");
    }

    #[test]
    fn session_state_payload_legacy_raw_bytes_rejected() {
        // Old workers uploaded raw session_id bytes (not JSON). The new
        // resumer's parse must reject this so it falls back to the primer.
        let bytes = b"claude-session-abc";
        let parsed: Result<SessionStatePayload, _> = serde_json::from_slice(bytes);
        assert!(
            parsed.is_err(),
            "legacy raw bytes must not parse as payload"
        );
    }

    mod graph_view {
        use super::*;
        use crate::graph::{GraphView, ObjectKind};
        use chrono::TimeZone;
        use serde_json::json;

        fn sample_conversation() -> Conversation {
            Conversation::new(
                ConversationId::new(),
                Some("Triaging flake".to_string()),
                Some(AgentName::try_new("claude").unwrap()),
                ConversationStatus::Active,
                Username::from("alice"),
                SessionSettings::default(),
                Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 2, 12, 0, 0).unwrap(),
            )
        }

        #[test]
        fn kind_is_conversation() {
            assert_eq!(<Conversation as GraphView>::KIND, ObjectKind::Conversation);
        }

        #[test]
        fn view_l1_matches_expected() {
            let conv = sample_conversation();
            assert_eq!(
                conv.view_l1(),
                json!({
                    "title": "Triaging flake",
                    "status": "active",
                })
            );
        }

        #[test]
        fn view_l2_matches_expected() {
            let conv = sample_conversation();
            assert_eq!(
                conv.view_l2(),
                json!({
                    "title": "Triaging flake",
                    "status": "active",
                    "agent_name": "claude",
                    "creator": "alice",
                    "updated_at": "2026-05-02T12:00:00Z",
                })
            );
        }

        #[test]
        fn view_l3_round_trips_to_original() {
            let conv = sample_conversation();
            let value = conv.view_l3();
            let roundtripped: Conversation = serde_json::from_value(value).unwrap();
            assert_eq!(roundtripped, conv);
        }

        #[test]
        fn view_l2_contains_view_l1_keys_with_same_values() {
            let conv = sample_conversation();
            let l1 = conv.view_l1();
            let l2 = conv.view_l2();
            for (key, expected) in l1.as_object().unwrap() {
                assert_eq!(l2.get(key), Some(expected), "key {key} mismatch in L2");
            }
        }
    }
}
