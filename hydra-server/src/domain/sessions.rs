use super::task_status::{Status, TaskError};
use super::users::Username;
use chrono::{DateTime, Utc};
use hydra_common::api::v1 as api;
use hydra_common::api::v1::sessions::{McpConfig, TokenUsage};
use hydra_common::{ConversationId, IssueId, RepoName, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn default_task_status() -> Status {
    Status::Complete
}

/// Settings that only apply when a session is running in interactive mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_resume_from: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub prompt: String,
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
    /// Interactive-only settings. `Some` for interactive sessions, `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<InteractiveOptions>,
    #[serde(default = "default_task_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
    /// Aggregated token usage reported by the worker at the end of a run.
    /// `None` until the worker submits a `Complete` status with usage data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: String,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        creator: Username,
        image: Option<String>,
        model: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
        mcp_config: Option<McpConfig>,
        interactive: Option<InteractiveOptions>,
        status: Status,
        last_message: Option<String>,
        error: Option<TaskError>,
    ) -> Self {
        Self {
            prompt,
            context,
            spawned_from,
            creator,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            mcp_config,
            interactive,
            status,
            last_message,
            error,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
            usage: None,
        }
    }

    /// Returns the conversation_id, if this is an interactive session attached
    /// to a conversation.
    pub fn conversation_id(&self) -> Option<&ConversationId> {
        self.interactive
            .as_ref()
            .and_then(|opts| opts.conversation_id.as_ref())
    }

    /// Returns `true` if this is an interactive session.
    pub fn is_interactive(&self) -> bool {
        self.interactive.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BundleSpec {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
    ServiceRepository {
        /// Name of a repository configured in the service configuration.
        name: RepoName,
        /// Optional git revision (branch, tag, or commit) to checkout after cloning.
        #[serde(default)]
        rev: Option<String>,
    },
}

impl Default for BundleSpec {
    fn default() -> Self {
        Self::None
    }
}

impl From<Bundle> for BundleSpec {
    fn from(bundle: Bundle) -> Self {
        match bundle {
            Bundle::None => BundleSpec::None,
            Bundle::GitRepository { url, rev } => BundleSpec::GitRepository { url, rev },
        }
    }
}

impl From<api::sessions::BundleSpec> for BundleSpec {
    fn from(value: api::sessions::BundleSpec) -> Self {
        match value {
            api::sessions::BundleSpec::None => BundleSpec::None,
            api::sessions::BundleSpec::GitRepository { url, rev } => {
                BundleSpec::GitRepository { url, rev }
            }
            api::sessions::BundleSpec::ServiceRepository { name, rev } => {
                BundleSpec::ServiceRepository { name, rev }
            }
            _ => unreachable!("unsupported bundle spec variant"),
        }
    }
}

impl From<BundleSpec> for api::sessions::BundleSpec {
    fn from(value: BundleSpec) -> Self {
        match value {
            BundleSpec::None => api::sessions::BundleSpec::None,
            BundleSpec::GitRepository { url, rev } => {
                api::sessions::BundleSpec::GitRepository { url, rev }
            }
            BundleSpec::ServiceRepository { name, rev } => {
                api::sessions::BundleSpec::ServiceRepository { name, rev }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Bundle {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
}

impl From<api::sessions::Bundle> for Bundle {
    fn from(value: api::sessions::Bundle) -> Self {
        match value {
            api::sessions::Bundle::None => Bundle::None,
            api::sessions::Bundle::GitRepository { url, rev } => Bundle::GitRepository { url, rev },
            _ => unreachable!("unsupported bundle variant"),
        }
    }
}

impl From<Bundle> for api::sessions::Bundle {
    fn from(value: Bundle) -> Self {
        match value {
            Bundle::None => api::sessions::Bundle::None,
            Bundle::GitRepository { url, rev } => api::sessions::Bundle::GitRepository { url, rev },
        }
    }
}

impl From<api::sessions::InteractiveOptions> for InteractiveOptions {
    fn from(value: api::sessions::InteractiveOptions) -> Self {
        InteractiveOptions {
            conversation_id: value.conversation_id,
            conversation_resume_from: value.conversation_resume_from,
        }
    }
}

impl From<InteractiveOptions> for api::sessions::InteractiveOptions {
    fn from(value: InteractiveOptions) -> Self {
        api::sessions::InteractiveOptions::new(
            value.conversation_id,
            None,
            value.conversation_resume_from,
        )
    }
}

impl TryFrom<api::sessions::Session> for Session {
    type Error = crate::domain::task_status::UnsupportedVariantError;

    fn try_from(value: api::sessions::Session) -> Result<Self, Self::Error> {
        Ok(Session {
            prompt: value.prompt,
            context: value.context.into(),
            spawned_from: value.spawned_from,
            creator: value.creator.into(),
            image: value.image,
            model: value.model,
            env_vars: value.env_vars,
            cpu_limit: value.cpu_limit,
            memory_limit: value.memory_limit,
            secrets: value.secrets,
            mcp_config: value.mcp_config,
            interactive: value.interactive.map(Into::into),
            status: value.status.try_into()?,
            last_message: value.last_message,
            error: value.error.map(TryInto::try_into).transpose()?,
            deleted: value.deleted,
            creation_time: value.creation_time,
            start_time: value.start_time,
            end_time: value.end_time,
            usage: value.usage,
        })
    }
}

impl From<Session> for api::sessions::Session {
    fn from(value: Session) -> Self {
        let mut session = api::sessions::Session::new(
            value.prompt,
            value.context.into(),
            value.spawned_from,
            value.creator.into(),
            value.image,
            value.model,
            value.env_vars,
            value.cpu_limit,
            value.memory_limit,
            value.secrets,
            value.mcp_config,
            value.interactive.map(Into::into),
            value.status.into(),
            value.last_message,
            value.error.map(Into::into),
            value.deleted,
            value.creation_time,
            value.start_time,
            value.end_time,
        );
        session.usage = value.usage;
        session
    }
}

/// Domain twin of [`api::sessions::SessionEvent`]. Append-only log of
/// model-context events for a session — the transcript the model "sees" is the
/// projection of this log onto `UserMessage` and `AssistantMessage` variants in
/// insertion order. Mirrors [`super::conversations::ConversationEvent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// User input received by the model.
    UserMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Assistant text emitted by the model.
    AssistantMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Tool-use event (call + result), captured for replay / debugging.
    ToolUse {
        tool_name: String,
        payload: Value,
        timestamp: DateTime<Utc>,
    },
    /// The worker is suspending the session.
    Suspending {
        reason: String,
        timestamp: DateTime<Utc>,
    },
    /// The model-context state was loaded from a prior session.
    Resumed {
        from_session_id: SessionId,
        timestamp: DateTime<Utc>,
    },
    /// Session is closed — no further events will be appended.
    Closed { timestamp: DateTime<Utc> },
}

impl SessionEvent {
    /// Returns a short preview string for this event, suitable for summaries.
    pub fn preview(&self) -> String {
        const MAX_LEN: usize = 100;

        fn truncate(content: &str, prefix: &str) -> String {
            let remaining = MAX_LEN.saturating_sub(prefix.len());
            if content.len() <= remaining {
                format!("{prefix}{content}")
            } else {
                let truncated: String = content.chars().take(remaining).collect();
                format!("{prefix}{truncated}…")
            }
        }

        match self {
            SessionEvent::UserMessage { content, .. } => truncate(content, "User: "),
            SessionEvent::AssistantMessage { content, .. } => truncate(content, "Assistant: "),
            SessionEvent::ToolUse { tool_name, .. } => format!("Tool: {tool_name}"),
            SessionEvent::Suspending { reason, .. } => format!("Suspending: {reason}"),
            SessionEvent::Resumed { .. } => "Resumed".to_string(),
            SessionEvent::Closed { .. } => "Closed".to_string(),
        }
    }
}

/// API → domain conversion. The forward-compat `Unknown` variant is unique to
/// the wire type; callers that receive it must handle it before downcasting to
/// the domain (typically by treating it as a versioning error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSessionEventVariant;

impl std::fmt::Display for UnknownSessionEventVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("session event has an unknown variant")
    }
}

impl std::error::Error for UnknownSessionEventVariant {}

impl TryFrom<api::sessions::SessionEvent> for SessionEvent {
    type Error = UnknownSessionEventVariant;

    fn try_from(value: api::sessions::SessionEvent) -> Result<Self, Self::Error> {
        Ok(match value {
            api::sessions::SessionEvent::UserMessage { content, timestamp } => {
                SessionEvent::UserMessage { content, timestamp }
            }
            api::sessions::SessionEvent::AssistantMessage { content, timestamp } => {
                SessionEvent::AssistantMessage { content, timestamp }
            }
            api::sessions::SessionEvent::ToolUse {
                tool_name,
                payload,
                timestamp,
            } => SessionEvent::ToolUse {
                tool_name,
                payload,
                timestamp,
            },
            api::sessions::SessionEvent::Suspending { reason, timestamp } => {
                SessionEvent::Suspending { reason, timestamp }
            }
            api::sessions::SessionEvent::Resumed {
                from_session_id,
                timestamp,
            } => SessionEvent::Resumed {
                from_session_id,
                timestamp,
            },
            api::sessions::SessionEvent::Closed { timestamp } => SessionEvent::Closed { timestamp },
            api::sessions::SessionEvent::Unknown => return Err(UnknownSessionEventVariant),
            _ => return Err(UnknownSessionEventVariant),
        })
    }
}

impl From<SessionEvent> for api::sessions::SessionEvent {
    fn from(value: SessionEvent) -> Self {
        match value {
            SessionEvent::UserMessage { content, timestamp } => {
                api::sessions::SessionEvent::UserMessage { content, timestamp }
            }
            SessionEvent::AssistantMessage { content, timestamp } => {
                api::sessions::SessionEvent::AssistantMessage { content, timestamp }
            }
            SessionEvent::ToolUse {
                tool_name,
                payload,
                timestamp,
            } => api::sessions::SessionEvent::ToolUse {
                tool_name,
                payload,
                timestamp,
            },
            SessionEvent::Suspending { reason, timestamp } => {
                api::sessions::SessionEvent::Suspending { reason, timestamp }
            }
            SessionEvent::Resumed {
                from_session_id,
                timestamp,
            } => api::sessions::SessionEvent::Resumed {
                from_session_id,
                timestamp,
            },
            SessionEvent::Closed { timestamp } => api::sessions::SessionEvent::Closed { timestamp },
        }
    }
}

/// Domain twin of [`api::sessions::SessionEventSummary`]. Mirrors
/// `ConversationEventSummary` so the eventual session-event store methods can
/// return the same shape per session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEventSummary {
    pub event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_preview: Option<String>,
}

impl From<api::sessions::SessionEventSummary> for SessionEventSummary {
    fn from(value: api::sessions::SessionEventSummary) -> Self {
        Self {
            event_count: value.event_count,
            last_event_preview: value.last_event_preview,
        }
    }
}

impl From<SessionEventSummary> for api::sessions::SessionEventSummary {
    fn from(value: SessionEventSummary) -> Self {
        api::sessions::SessionEventSummary::new(value.event_count, value.last_event_preview)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BundleSpec, Session, SessionEvent, SessionEventSummary, UnknownSessionEventVariant,
    };
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use chrono::Utc;
    use hydra_common::api::v1 as api;
    use hydra_common::{RepoName, SessionId};
    use std::collections::HashMap;
    use std::str::FromStr;

    #[test]
    fn bundle_spec_converts_between_domain_and_api() {
        let repo = RepoName::from_str("dourolabs/hydra").unwrap();
        let domain = BundleSpec::ServiceRepository {
            name: repo.clone(),
            rev: Some("main".to_string()),
        };

        let api_spec: api::sessions::BundleSpec = domain.clone().into();
        let round_trip: BundleSpec = api_spec.into();

        assert_eq!(round_trip, domain);
    }

    #[test]
    fn session_roundtrip_preserves_secrets() {
        let secrets = Some(vec!["db-secret".to_string(), "api-key".to_string()]);
        let domain_session = Session::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            Some("worker:latest".to_string()),
            Some("gpt-4o".to_string()),
            HashMap::new(),
            Some("400m".to_string()),
            Some("768Mi".to_string()),
            secrets.clone(),
            None,
            None,
            Status::Created,
            None,
            None,
        );

        let api_session: api::sessions::Session = domain_session.clone().into();
        let round_trip: Session = api_session.try_into().unwrap();

        assert_eq!(round_trip.secrets, secrets);
        assert_eq!(round_trip.prompt, domain_session.prompt);
        assert_eq!(round_trip.image, domain_session.image);
        assert_eq!(round_trip.model, domain_session.model);
    }

    #[test]
    fn session_roundtrip_preserves_empty_secrets() {
        let domain_session = Session::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        );

        let api_session: api::sessions::Session = domain_session.clone().into();
        let round_trip: Session = api_session.try_into().unwrap();

        assert_eq!(round_trip.secrets, None);
    }

    fn round_trip_session_event(event: api::sessions::SessionEvent) {
        let domain: SessionEvent = event.clone().try_into().expect("known variant");
        let back: api::sessions::SessionEvent = domain.into();
        assert_eq!(back, event);
    }

    #[test]
    fn session_event_user_message_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_assistant_message_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::AssistantMessage {
            content: "hi there".to_string(),
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_tool_use_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::ToolUse {
            tool_name: "browser_navigate".to_string(),
            payload: serde_json::json!({"url": "https://example.test"}),
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_suspending_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::Suspending {
            reason: "idle_timeout".to_string(),
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_resumed_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::Resumed {
            from_session_id: SessionId::new(),
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_closed_round_trips_through_domain() {
        round_trip_session_event(api::sessions::SessionEvent::Closed {
            timestamp: Utc::now(),
        });
    }

    #[test]
    fn session_event_unknown_variant_is_rejected_at_domain_boundary() {
        let result: Result<SessionEvent, _> = api::sessions::SessionEvent::Unknown.try_into();
        assert_eq!(result.unwrap_err(), UnknownSessionEventVariant);
    }

    #[test]
    fn session_event_summary_round_trips() {
        let api_summary = api::sessions::SessionEventSummary::new(3, Some("preview".to_string()));
        let domain: SessionEventSummary = api_summary.clone().into();
        let back: api::sessions::SessionEventSummary = domain.into();
        assert_eq!(back, api_summary);
    }
}
