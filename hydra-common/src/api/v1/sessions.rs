use crate::{
    BuildCacheContext, ConversationId, IssueId, RepoName, SessionId, VersionNumber,
    actor_ref::ActorRef,
    api::v1::{agents::AgentName, projects::StatusKey, timeout::Timeout},
    task_status::{Status, TaskError},
    users::Username,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// MCP (Model Context Protocol) server configuration.
///
/// Stored as a JSON object to remain flexible as the MCP config schema evolves.
pub type McpConfig = Value;

/// A TCP port the worker has advertised as serving an HTTP target the user
/// (or the platform's reverse proxy) can reach. Recorded on the session via
/// `hydra worker proxy start --port <N>` and removed via
/// `hydra worker proxy stop  --port <N>`.
///
/// `ready_path`, when set, is the path the proxy should `GET` to confirm the
/// server inside the worker is actually serving requests before forwarding
/// user traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ProxyTarget {
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_path: Option<String>,
}

/// Aggregated token totals reported by the worker at the end of a session run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

/// Per-session knobs that the worker hands to the model wrapper.
///
/// Spelled out as its own struct (not flattened into `Session`) so each
/// field carries a single, unambiguous meaning. `system_prompt` is
/// resolved server-side from the agent definition; historical rows
/// loaded through the legacy backfill path leave it `None`.
///
/// `agent_name` is typed as `Option<AgentName>` so the agent-vs-adhoc
/// discriminant on a session is a validated type, not a free string:
/// `Some(name)` is agent-spawned, `None` is ad-hoc. Historic rows with a
/// malformed `agent_name` fail to deserialize loudly rather than silently
/// passing through.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AgentConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<AgentName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
}

impl AgentConfig {
    pub fn new(
        agent_name: Option<AgentName>,
        model: Option<String>,
        system_prompt: Option<String>,
        mcp_config: Option<McpConfig>,
    ) -> Self {
        Self {
            agent_name,
            model,
            system_prompt,
            mcp_config,
        }
    }
}

/// First-class discriminant for the two kinds of sessions Hydra runs.
///
/// A session is in exactly one mode at a time; making the mode an enum
/// kills the previous `(prompt, interactive)` cross-field validation.
/// Resumption is **not** a mode — it's the lineage edge
/// `Session::resumed_from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionMode {
    /// One-shot headless task. The prompt is sourced from
    /// `Session::agent_config.system_prompt`.
    Headless,
    /// Interactive session attached to a conversation.
    Interactive {
        conversation_id: ConversationId,
        /// Worker-side idle timeout override. `None` means the server
        /// applies its configured default (`job.interactive_idle_timeout_secs`)
        /// at handshake time — used when the caller didn't supply a value
        /// and for legacy rows that don't carry one.
        /// `Some(Timeout::Infinite)` means the worker never arms an idle
        /// clock for this session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        idle_timeout: Option<Timeout>,
        /// Whether the agent should produce a greeting turn before any user
        /// message arrives. When `true`, the server emits `FirstMessage` with
        /// an empty `user_message` as soon as the worker signals `Ready`,
        /// rather than waiting on the conversation's first user message.
        #[serde(default)]
        greet_user: bool,
    },
}

impl SessionMode {
    /// Convenience accessor for the linked conversation (`None` on headless).
    pub fn conversation_id(&self) -> Option<&ConversationId> {
        match self {
            SessionMode::Headless => None,
            SessionMode::Interactive {
                conversation_id, ..
            } => Some(conversation_id),
        }
    }

    /// `true` iff `mode` is `Interactive` with `greet_user = true`.
    pub fn greet_user(&self) -> bool {
        matches!(
            self,
            SessionMode::Interactive {
                greet_user: true,
                ..
            }
        )
    }

    /// Tag-only discriminant — the kind of session, without any payload.
    pub fn kind(&self) -> SessionModeKind {
        match self {
            SessionMode::Headless => SessionModeKind::Headless,
            SessionMode::Interactive { .. } => SessionModeKind::Interactive,
        }
    }
}

/// Tag-only mirror of [`SessionMode`] — carries no payload, just the
/// discriminant. Used by [`WorkerContext`] so the worker knows which
/// dispatch path (`drive_headless` vs. `drive_interactive`) to take
/// without seeing the conversation id / resume index / greet_user fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
pub enum SessionModeKind {
    Headless,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Session {
    // === Universal identity / lineage ===
    pub creator: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    /// Predecessor session for resumed runs. `None` for fresh sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<SessionId>,

    // === Universal agent inputs ===
    #[serde(default)]
    pub agent_config: AgentConfig,
    /// Server-supplied mount layout. Mandatory: no serde default;
    /// deserialization fails loudly if the field is missing, so client
    /// requests can't silently fall back to an empty layout.
    pub mount_spec: MountSpec,

    // === Universal runtime knobs ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,

    // === Mode (first-class) ===
    /// Mandatory: no serde default; deserialization fails loudly if the
    /// field is missing rather than picking an arbitrary mode.
    pub mode: SessionMode,

    // === Universal lifecycle ===
    #[serde(default = "default_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
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
    /// Ports the worker has advertised as HTTP targets for the interactive
    /// dev preview. Edited via `hydra worker proxy {start,stop}` and read by
    /// the proxy reach path (PR 2). Default empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proxy_targets: Vec<ProxyTarget>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        creator: Username,
        spawned_from: Option<IssueId>,
        resumed_from: Option<SessionId>,
        agent_config: AgentConfig,
        mount_spec: MountSpec,
        image: Option<String>,
        env_vars: HashMap<String, String>,
        cpu_limit: Option<String>,
        memory_limit: Option<String>,
        secrets: Option<Vec<String>>,
        mode: SessionMode,
        status: Status,
        last_message: Option<String>,
        error: Option<TaskError>,
        deleted: bool,
        creation_time: Option<DateTime<Utc>>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            creator,
            spawned_from,
            resumed_from,
            agent_config,
            mount_spec,
            image,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            mode,
            status,
            last_message,
            error,
            deleted,
            creation_time,
            start_time,
            end_time,
            usage: None,
            proxy_targets: Vec::new(),
        }
    }

    /// The linked conversation, if this is an interactive session.
    pub fn conversation_id(&self) -> Option<&ConversationId> {
        self.mode.conversation_id()
    }

    /// `true` iff `mode` is `SessionMode::Interactive`.
    pub fn is_interactive(&self) -> bool {
        matches!(self.mode, SessionMode::Interactive { .. })
    }
}

fn default_status() -> Status {
    Status::Created
}

/// Caller-facing selector for how to build the resulting
/// [`Session::agent_config`].
///
/// `Named` defers to the server: the request carries only the agent name,
/// and `AppState::create_session` looks up the agent row to resolve the
/// `system_prompt` / `mcp_config` / `secrets` / `AGENT_NAME_ENV_VAR` env
/// var. `Adhoc` skips the DB lookup entirely — the caller hands the
/// server a literal prompt (non-optional on the wire) and an optional
/// inline `mcp_config`.
///
/// The two variants are exclusive: there is no "Named with prompt
/// override" middle ground. See `i-mnmvcxmd` for the design rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentSpec {
    /// Server resolves prompt / mcp_config / secrets from the agent row.
    Named { name: AgentName },
    /// Caller supplies the prompt and (optional) mcp_config inline.
    Adhoc {
        system_prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_config: Option<McpConfig>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CreateSessionRequest {
    pub mode: SessionMode,
    pub agent_config: AgentSpec,
    /// Model override that applies to both `Named` and `Adhoc` variants.
    /// Sibling of `image` — same kind of orthogonal override knob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub mount_spec: MountSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_vars: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Bundle {
    #[serde(rename = "none")]
    None,
    GitRepository {
        /// Remote Git repository URL that should be cloned for the session context.
        url: String,
        /// Specific git revision (branch, tag, or commit) to checkout after cloning.
        rev: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
}

impl<'de> Deserialize<'de> for Bundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleHelper>(value) {
            Ok(BundleHelper::None) => Ok(Bundle::None),
            Ok(BundleHelper::GitRepository { url, rev }) => Ok(Bundle::GitRepository { url, rev }),
            Err(_) => Ok(Bundle::Unknown),
        }
    }
}

/// A relative, non-traversing filesystem path used in [`MountSpec`] /
/// [`MountItem`] to describe where mounts land under the worker's per-job
/// `dest` directory. Construction and deserialization reject absolute paths
/// and any `..` component, so a server payload cannot point the worker at
/// `/etc` or `../escape`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
#[serde(transparent)]
pub struct RelativePath(PathBuf);

/// Error returned when a [`RelativePath`] fails validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelativePathError {
    /// Path is absolute (e.g. `/etc`).
    Absolute,
    /// Path contains a `..` component (e.g. `foo/../bar`).
    ParentTraversal,
    /// Path is empty.
    Empty,
}

impl std::fmt::Display for RelativePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absolute => f.write_str("path must be relative, not absolute"),
            Self::ParentTraversal => f.write_str("path must not contain `..` components"),
            Self::Empty => f.write_str("path must not be empty"),
        }
    }
}

impl std::error::Error for RelativePathError {}

impl RelativePath {
    /// Build a `RelativePath`, rejecting absolute paths and any `..` component.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, RelativePathError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(RelativePathError::Empty);
        }
        if path.is_absolute() {
            return Err(RelativePathError::Absolute);
        }
        for component in path.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => return Err(RelativePathError::ParentTraversal),
                Component::Prefix(_) | Component::RootDir => {
                    return Err(RelativePathError::Absolute);
                }
            }
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for RelativePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        RelativePath::new(s).map_err(serde::de::Error::custom)
    }
}

/// Server-side description of the worker's filesystem layout.
///
/// `working_dir` is where the agent's CWD will live (relative to the worker's
/// `dest` root); `mounts` is the ordered list of mounts to set up before the
/// agent runs and tear down / persist after it finishes. One [`MountItem`] in
/// this vec corresponds to one mount on the worker side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct MountSpec {
    pub working_dir: RelativePath,
    pub mounts: Vec<MountItem>,
}

impl MountSpec {
    pub fn new(working_dir: RelativePath, mounts: Vec<MountItem>) -> Self {
        Self {
            working_dir,
            mounts,
        }
    }

    /// `true` when the spec carries no mounts. Used by the server-side
    /// `CreateSession` handler to decide whether to apply
    /// `session_settings`-derived defaults from a `spawned_from` issue.
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }
}

impl Default for MountSpec {
    fn default() -> Self {
        Self {
            working_dir: RelativePath::new("repo").expect("static `repo` is valid"),
            mounts: Vec::new(),
        }
    }
}

/// One mount's worth of server-supplied configuration.
///
/// Each variant is a pure intent the server hands to the worker: it names
/// what to mount and where, but carries no session identity. The worker
/// supplies the session-id and issue-branch-id at instantiation time from
/// its own `WorkerContext`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum MountItem {
    /// Materialize a bundle (git repo or empty) at `target` under the worker
    /// dest dir.
    Bundle {
        target: RelativePath,
        bundle: Bundle,
    },

    /// Apply / upload the nearest build cache against `repo_target`.
    BuildCache {
        repo_target: RelativePath,
        service_repo_name: RepoName,
        context: BuildCacheContext,
    },

    /// Sync / push the Hydra document store into `target`.
    Documents { target: RelativePath },

    /// Forward-compat fallback. Old clients reading a spec that contains an
    /// unrecognized item tag deserialize the item as `Unknown` so the rest of
    /// the vec is still understood. The worker treats any `Unknown` item as a
    /// fatal "client is too old for this spec" error.
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
enum MountItemHelper {
    Bundle {
        target: RelativePath,
        bundle: Bundle,
        // Tolerated for backward compatibility with persisted `tasks_v2.mount_spec`
        // JSON rows written before PR-D moved session metadata off `MountItem`.
        // Silently discarded — `InstantiateInputs` now sources these values.
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<SessionId>,
        #[serde(default)]
        #[allow(dead_code)]
        issue_branch_id: Option<String>,
    },
    BuildCache {
        repo_target: RelativePath,
        service_repo_name: RepoName,
        context: BuildCacheContext,
        // Same backward-compat tolerance as `Bundle::session_id` above.
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<SessionId>,
    },
    Documents {
        target: RelativePath,
    },
}

impl<'de> Deserialize<'de> for MountItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<MountItemHelper>(value) {
            Ok(MountItemHelper::Bundle { target, bundle, .. }) => {
                Ok(MountItem::Bundle { target, bundle })
            }
            Ok(MountItemHelper::BuildCache {
                repo_target,
                service_repo_name,
                context,
                ..
            }) => Ok(MountItem::BuildCache {
                repo_target,
                service_repo_name,
                context,
            }),
            Ok(MountItemHelper::Documents { target }) => Ok(MountItem::Documents { target }),
            Err(_) => Ok(MountItem::Unknown),
        }
    }
}

/// Orchestration-only context handed to the worker at boot.
///
/// Carries everything the worker needs to set up its execution
/// environment, plus enough metadata (`session_id`, `mode_kind`) to drive
/// the websocket lifecycle. Anything the model itself sees (system
/// prompt, transcript, resume blob) is intentionally absent here and
/// arrives over the relay websocket instead, so the worker has a single
/// channel for all model-input data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkerContext {
    pub session_id: SessionId,
    pub mode_kind: SessionModeKind,
    pub mounts: Vec<MountItem>,
    pub working_dir: RelativePath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
    /// Idle timeout for the worker's interactive event loop. `None` is
    /// only emitted for headless sessions and for interactive sessions
    /// where the resolved timeout is `Timeout::Infinite` — in both
    /// cases the worker must not arm an idle clock. `Some(seconds)`
    /// arms the clock to that many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout: Option<Timeout>,
    #[serde(default)]
    pub resolved_env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,
}

impl WorkerContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        mode_kind: SessionModeKind,
        mounts: Vec<MountItem>,
        working_dir: RelativePath,
        model: Option<String>,
        mcp_config: Option<McpConfig>,
        idle_timeout: Option<Timeout>,
        resolved_env: HashMap<String, String>,
        github_token: Option<String>,
    ) -> Self {
        Self {
            session_id,
            mode_kind,
            mounts,
            working_dir,
            model,
            mcp_config,
            idle_timeout,
            resolved_env,
            github_token,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateSessionResponse {
    pub session_id: SessionId,
    /// Fully-lowered persisted session. Lets callers skip a follow-up
    /// `GET /v1/sessions/:id`.
    pub session: Session,
}

impl CreateSessionResponse {
    pub fn new(session_id: SessionId, session: Session) -> Self {
        Self {
            session_id,
            session,
        }
    }
}

/// Lightweight summary of a session for list views.
///
/// Excludes `context`, `image`, `model`, `env_vars`, `cpu_limit`,
/// `memory_limit`, `secrets`, `last_message`, and the full `interactive`
/// options (only the linked `conversation_id` is exposed). The aggregated
/// `usage` totals reported by the worker are included.
/// The `prompt` field is truncated to the first 20 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSummary {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    pub creator: Username,
    #[serde(default = "default_status")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
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

impl From<&Session> for SessionSummary {
    fn from(session: &Session) -> Self {
        let raw_prompt = session
            .agent_config
            .system_prompt
            .as_deref()
            .unwrap_or_default();
        let prompt = if raw_prompt.chars().count() > 20 {
            let mut s: String = raw_prompt.chars().take(20).collect();
            s.push_str("...");
            s
        } else {
            raw_prompt.to_string()
        };
        let error = session.error.as_ref().map(|e| match e {
            TaskError::JobEngineError { reason } => {
                if reason.chars().count() > 100 {
                    let truncated: String = reason.chars().take(100).collect();
                    TaskError::JobEngineError {
                        reason: truncated + "...",
                    }
                } else {
                    e.clone()
                }
            }
            _ => e.clone(),
        });
        SessionSummary {
            prompt,
            spawned_from: session.spawned_from.clone(),
            conversation_id: session.conversation_id().cloned(),
            creator: session.creator.clone(),
            status: session.status,
            error,
            deleted: session.deleted,
            creation_time: session.creation_time,
            start_time: session.start_time,
            end_time: session.end_time,
            usage: session.usage.clone(),
        }
    }
}

/// Summary-level version record for session list responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSummaryRecord {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    #[serde(alias = "task")]
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl From<&SessionVersionRecord> for SessionSummaryRecord {
    fn from(record: &SessionVersionRecord) -> Self {
        SessionSummaryRecord {
            session_id: record.session_id.clone(),
            version: record.version,
            timestamp: record.timestamp,
            session: SessionSummary::from(&record.session),
            actor: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListSessionsResponse {
    #[serde(alias = "jobs")]
    pub sessions: Vec<SessionSummaryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

impl ListSessionsResponse {
    pub fn new(sessions: Vec<SessionSummaryRecord>) -> Self {
        Self {
            sessions,
            next_cursor: None,
            total_count: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionVersionRecord {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    #[serde(alias = "task")]
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl SessionVersionRecord {
    pub fn new(
        session_id: SessionId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        session: Session,
        actor: Option<ActorRef>,
    ) -> Self {
        Self {
            session_id,
            version,
            timestamp,
            session,
            actor,
        }
    }
}

use super::serde_helpers::{
    deserialize_comma_separated, deserialize_comma_separated_json, serialize_comma_separated,
    serialize_comma_separated_json,
};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchSessionsQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_from: Option<IssueId>,
    /// Filter sessions spawned from any of these issue IDs (comma-separated, max 100).
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub spawned_from_ids: Vec<IssueId>,
    #[serde(default)]
    pub include_deleted: Option<bool>,
    /// Filter sessions by creator username.
    #[serde(default)]
    pub creator: Option<String>,
    /// Filter sessions by the interactive conversation they are attached to.
    /// Only interactive sessions whose `interactive.conversation_id` matches
    /// are returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Filter sessions by status (comma-separated in query string). When multiple
    /// statuses are provided, a session matches if its status is any of the given values.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated_json",
        deserialize_with = "deserialize_comma_separated_json"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub status: Vec<Status>,
    /// Maximum number of results to return. When omitted, all results are returned.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque cursor from a previous response's `next_cursor` field.
    #[serde(default)]
    pub cursor: Option<String>,
    /// When true, include `total_count` in the response.
    #[serde(default)]
    pub count: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListSessionVersionsResponse {
    pub versions: Vec<SessionVersionRecord>,
}

impl ListSessionVersionsResponse {
    pub fn new(versions: Vec<SessionVersionRecord>) -> Self {
        Self { versions }
    }
}

impl SearchSessionsQuery {
    pub fn new(
        q: Option<String>,
        spawned_from: Option<IssueId>,
        include_deleted: Option<bool>,
        status: Vec<Status>,
    ) -> Self {
        Self {
            q,
            spawned_from,
            spawned_from_ids: Vec::new(),
            include_deleted,
            creator: None,
            conversation_id: None,
            status,
            limit: None,
            cursor: None,
            count: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct KillSessionResponse {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub status: String,
}

impl KillSessionResponse {
    pub fn new(session_id: SessionId, status: String) -> Self {
        Self { session_id, status }
    }
}

/// Body for `POST /v1/sessions/<sid>/proxy-targets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UpsertProxyTargetRequest {
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_path: Option<String>,
}

/// Body for `GET /v1/sessions/<sid>/proxy-targets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ListProxyTargetsResponse {
    pub targets: Vec<ProxyTarget>,
}

/// Which restore path produced a [`SessionEvent::Resumed`] event.
///
/// The worker tries native materialization first; on failure it falls back
/// to replaying the prior session's transcript as primer text. Both paths
/// reach steady state and the worker emits `Resumed` either way — this
/// discriminator lets downstream consumers (dashboards, debug tooling) tell
/// the two apart without inspecting other fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
pub enum ResumeSource {
    /// The new worker materialized a `resume_blob` from the prior session
    /// (e.g. uploaded `session_state`) and continued in-place.
    Native,
    /// No usable `resume_blob` was available, so the new worker primed itself
    /// by replaying the prior session's transcript as text. This is the path
    /// that real close→reopen→send flows hit today.
    Transcript,
}

/// Append-only log of model-context events for a session. The transcript the
/// model "sees" is the projection of this log onto `UserMessage` and
/// `AssistantMessage` variants in insertion order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionEvent {
    /// User input received by the model. For headless sessions this is the
    /// initial prompt (and any future tool-supplied inputs); for interactive
    /// sessions this is each user turn from the relay.
    UserMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Assistant text emitted by the model.
    AssistantMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// Tool-use event (call + result) captured for replay / debugging; not part
    /// of the resumable transcript by default. `payload` is structured but
    /// model-agnostic.
    ToolUse {
        tool_name: String,
        payload: Value,
        timestamp: DateTime<Utc>,
    },
    /// The worker is suspending the session (idle timeout, kill signal, etc.).
    /// The next event is typically a `Closed` on the same session or a
    /// `Resumed` on the next session.
    Suspending {
        reason: String,
        timestamp: DateTime<Utc>,
    },
    /// The model-context state was loaded from a prior session. Always the
    /// first event on a resumed session; carries the predecessor session id
    /// and a [`ResumeSource`] flag distinguishing the native-materialization
    /// path from the transcript-replay fallback.
    Resumed {
        from_session_id: SessionId,
        source: ResumeSource,
        timestamp: DateTime<Utc>,
    },
    /// Session is closed — no further events will be appended.
    Closed { timestamp: DateTime<Utc> },
    /// System-originated event surfaced to the agent (e.g. a child issue
    /// reaching an unblocking status). The worker re-injects this as a
    /// user-shaped input by calling [`SystemEventKind::render`]; the
    /// frontend renders the structured form distinctly.
    SystemEvent {
        kind: SystemEventKind,
        timestamp: DateTime<Utc>,
    },
    /// Forward-compat fallback. Old clients reading an event whose `type` tag
    /// is unrecognized deserialize it as `Unknown` rather than erroring.
    #[serde(other)]
    Unknown,
}

impl SessionEvent {
    /// The event's own wall-clock timestamp, if any. `Unknown` carries no
    /// timestamp because the discriminator was opaque.
    pub fn timestamp(&self) -> Option<DateTime<Utc>> {
        match self {
            SessionEvent::UserMessage { timestamp, .. }
            | SessionEvent::AssistantMessage { timestamp, .. }
            | SessionEvent::ToolUse { timestamp, .. }
            | SessionEvent::Suspending { timestamp, .. }
            | SessionEvent::Resumed { timestamp, .. }
            | SessionEvent::Closed { timestamp }
            | SessionEvent::SystemEvent { timestamp, .. } => Some(*timestamp),
            SessionEvent::Unknown => None,
        }
    }
}

/// Structured kinds for [`SessionEvent::SystemEvent`]. Each variant has
/// exactly one canonical [`Self::render`] string — callers MUST NOT
/// compose their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SystemEventKind {
    /// A child issue transitioned into a status that unblocks its parent;
    /// the parent's conversation surface is being woken so the agent can
    /// continue.
    ChildUnblocked {
        child_id: IssueId,
        new_status: StatusKey,
    },
}

impl SystemEventKind {
    /// Canonical string rendering for projection into a model's input
    /// stream. One canonical form per variant — callers MUST NOT compose
    /// their own.
    pub fn render(&self) -> String {
        match self {
            Self::ChildUnblocked {
                child_id,
                new_status,
            } => format!("Child {child_id} reached status {new_status}; please continue."),
        }
    }
}

/// Summary of session events for batch fetching, returned by
/// `get_session_event_summaries`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionEventSummary {
    pub event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_preview: Option<String>,
}

impl SessionEventSummary {
    pub fn new(event_count: usize, last_event_preview: Option<String>) -> Self {
        Self {
            event_count,
            last_event_preview,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueId, test_helpers::serialize_query_params};
    use std::collections::HashMap;

    #[test]
    fn search_sessions_query_serializes_with_reqwest() {
        let issue_id = IssueId::new();
        let query = SearchSessionsQuery {
            q: Some("test query".to_string()),
            spawned_from: Some(issue_id.clone()),
            spawned_from_ids: vec![],
            include_deleted: None,
            creator: None,
            conversation_id: None,
            status: vec![],
            limit: None,
            cursor: None,
            count: None,
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("q").map(String::as_str), Some("test query"));
        assert_eq!(
            params.get("spawned_from").map(String::as_str),
            Some(issue_id.as_ref())
        );
    }

    #[test]
    fn search_sessions_query_serializes_status_filter() {
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Running]);

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("status").map(String::as_str), Some("running"));
    }

    #[test]
    fn search_sessions_query_serializes_multi_status_filter() {
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![Status::Created, Status::Pending, Status::Running],
        );

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("status").map(String::as_str),
            Some("created,pending,running")
        );
    }

    #[test]
    fn search_sessions_query_deserializes_comma_separated_status() {
        let query: SearchSessionsQuery =
            serde_urlencoded::from_str("status=created%2Cpending%2Crunning").unwrap();
        assert_eq!(
            query.status,
            vec![Status::Created, Status::Pending, Status::Running]
        );
    }

    #[test]
    fn search_sessions_query_serializes_spawned_from_ids() {
        let id1 = IssueId::new();
        let id2 = IssueId::new();
        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        let query = SearchSessionsQuery {
            spawned_from_ids: vec![id1.clone(), id2.clone()],
            ..query
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        let expected = format!("{id1},{id2}");
        assert_eq!(
            params.get("spawned_from_ids").map(String::as_str),
            Some(expected.as_str())
        );
    }

    #[test]
    fn search_sessions_query_deserializes_spawned_from_ids() {
        let query: SearchSessionsQuery =
            serde_urlencoded::from_str("spawned_from_ids=i-abcd%2Ci-efgh").unwrap();
        assert_eq!(query.spawned_from_ids.len(), 2);
        assert_eq!(query.spawned_from_ids[0].as_ref(), "i-abcd");
        assert_eq!(query.spawned_from_ids[1].as_ref(), "i-efgh");
    }

    #[test]
    fn search_sessions_query_round_trips_creator() {
        let query = SearchSessionsQuery {
            creator: Some("alice".to_string()),
            ..SearchSessionsQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("creator").map(String::as_str), Some("alice"));

        let parsed: SearchSessionsQuery = serde_urlencoded::from_str("creator=alice").unwrap();
        assert_eq!(parsed.creator.as_deref(), Some("alice"));
    }

    #[test]
    fn search_sessions_query_round_trips_conversation_id() {
        let conv_id = crate::ConversationId::new();
        let query = SearchSessionsQuery {
            conversation_id: Some(conv_id.clone()),
            ..SearchSessionsQuery::default()
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            params.get("conversation_id").map(String::as_str),
            Some(conv_id.as_ref())
        );

        let encoded = format!("conversation_id={}", conv_id.as_ref());
        let parsed: SearchSessionsQuery = serde_urlencoded::from_str(&encoded).unwrap();
        assert_eq!(parsed.conversation_id.as_ref(), Some(&conv_id));
    }

    #[test]
    fn search_sessions_query_omits_conversation_id_when_unset() {
        let query = SearchSessionsQuery::default();
        let params = serialize_query_params(&query);
        assert!(
            params.iter().all(|(k, _)| k != "conversation_id"),
            "expected no conversation_id param when unset"
        );
    }

    #[test]
    fn search_sessions_query_serializes_empty_query() {
        let query = SearchSessionsQuery::default();

        let params = serialize_query_params(&query);
        assert!(
            params.is_empty(),
            "expected no query params for empty SearchSessionsQuery"
        );
    }

    fn make_test_session(prompt: &str) -> Session {
        Session::new(
            Username::from("alice"),
            Some(IssueId::new()),
            None,
            AgentConfig::new(
                None,
                Some("claude-3".to_string()),
                Some(prompt.to_string()),
                None,
            ),
            test_mount_spec(),
            Some("worker:latest".to_string()),
            HashMap::from([("KEY".to_string(), "val".to_string())]),
            Some("500m".to_string()),
            Some("1Gi".to_string()),
            Some(vec!["secret".to_string()]),
            SessionMode::Headless,
            Status::Running,
            Some("last message text".to_string()),
            None,
            false,
            Some(chrono::Utc::now()),
            Some(chrono::Utc::now()),
            None,
        )
    }

    fn test_mount_spec() -> MountSpec {
        MountSpec::new(
            RelativePath::new("repo").unwrap(),
            vec![MountItem::Documents {
                target: RelativePath::new("documents").unwrap(),
            }],
        )
    }

    #[test]
    fn session_summary_truncates_long_prompt() {
        let long_prompt = "x".repeat(500);
        let session = make_test_session(&long_prompt);
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, format!("{}...", "x".repeat(20)));
    }

    #[test]
    fn session_summary_preserves_short_prompt() {
        let session = make_test_session("short prompt");
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, "short prompt");
    }

    #[test]
    fn session_summary_excludes_heavy_fields() {
        let session = make_test_session("test prompt");
        let summary = SessionSummary::from(&session);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("context").is_none());
        assert!(value.get("image").is_none());
        assert!(value.get("model").is_none());
        assert!(value.get("env_vars").is_none());
        assert!(value.get("cpu_limit").is_none());
        assert!(value.get("memory_limit").is_none());
        assert!(value.get("secrets").is_none());
        assert!(value.get("last_message").is_none());
    }

    #[test]
    fn session_summary_maps_all_fields() {
        let session = make_test_session("my prompt");
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.prompt, "my prompt");
        assert!(summary.spawned_from.is_some());
        assert_eq!(summary.creator, Username::from("alice"));
        assert_eq!(summary.status, Status::Running);
        assert!(summary.error.is_none());
        assert!(!summary.deleted);
        assert!(summary.creation_time.is_some());
        assert!(summary.start_time.is_some());
        assert!(summary.end_time.is_none());
        // One-shot session has no `interactive`, so no linked conversation.
        assert!(summary.conversation_id.is_none());
        // The fixture leaves usage unset.
        assert!(summary.usage.is_none());
    }

    #[test]
    fn session_summary_populates_usage_when_present() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_input_tokens: 50,
            cache_creation_input_tokens: 25,
        };
        let mut session = make_test_session("prompt");
        session.usage = Some(usage.clone());
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.usage.as_ref(), Some(&usage));

        let value = serde_json::to_value(&summary).unwrap();
        let usage_value = value.get("usage").expect("usage present in json");
        assert_eq!(usage_value.get("input_tokens").unwrap(), 100);
        assert_eq!(usage_value.get("output_tokens").unwrap(), 200);
        assert_eq!(usage_value.get("cache_read_input_tokens").unwrap(), 50);
        assert_eq!(usage_value.get("cache_creation_input_tokens").unwrap(), 25);
    }

    #[test]
    fn session_summary_omits_usage_when_absent() {
        let session = make_test_session("prompt");
        let summary = SessionSummary::from(&session);
        assert!(summary.usage.is_none());
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("usage").is_none());
    }

    #[test]
    fn session_summary_includes_conversation_id_from_interactive() {
        let conv_id = crate::ConversationId::new();
        let mut session = make_test_session("interactive prompt");
        session.mode = SessionMode::Interactive {
            conversation_id: conv_id.clone(),
            idle_timeout: Timeout::seconds(600),
            greet_user: false,
        };
        let summary = SessionSummary::from(&session);
        assert_eq!(summary.conversation_id.as_ref(), Some(&conv_id));

        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(
            value.get("conversation_id").and_then(|v| v.as_str()),
            Some(conv_id.as_ref())
        );
    }

    #[test]
    fn session_summary_omits_conversation_id_when_absent() {
        let session = make_test_session("one-shot prompt");
        let summary = SessionSummary::from(&session);
        assert!(summary.conversation_id.is_none());
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("conversation_id").is_none());
    }

    #[test]
    fn session_summary_record_from_version_record() {
        let session = make_test_session("record test");
        let session_id = crate::SessionId::new();
        let record =
            SessionVersionRecord::new(session_id.clone(), 7, chrono::Utc::now(), session, None);
        let summary_record = SessionSummaryRecord::from(&record);
        assert_eq!(summary_record.session_id, session_id);
        assert_eq!(summary_record.version, 7);
        assert_eq!(summary_record.session.prompt, "record test");
        assert_eq!(summary_record.actor, None);
    }

    #[test]
    fn session_summary_truncates_long_error_reason() {
        let long_reason = "e".repeat(200);
        let mut session = make_test_session("prompt");
        session.error = Some(TaskError::JobEngineError {
            reason: long_reason,
        });
        let summary = SessionSummary::from(&session);
        let error = summary.error.unwrap();
        match error {
            TaskError::JobEngineError { reason } => {
                assert_eq!(reason.chars().count(), 103);
                assert!(reason.ends_with("..."));
                assert_eq!(&reason[..100], &"e".repeat(100));
            }
            _ => panic!("expected JobEngineError"),
        }
    }

    #[test]
    fn session_summary_preserves_short_error_reason() {
        let short_reason = "something went wrong".to_string();
        let mut session = make_test_session("prompt");
        session.error = Some(TaskError::JobEngineError {
            reason: short_reason.clone(),
        });
        let summary = SessionSummary::from(&session);
        let error = summary.error.unwrap();
        match error {
            TaskError::JobEngineError { reason } => {
                assert_eq!(reason, short_reason);
            }
            _ => panic!("expected JobEngineError"),
        }
    }

    #[test]
    fn session_summary_record_omits_actor() {
        let session = make_test_session("actor test");
        let session_id = crate::SessionId::new();
        let actor = ActorRef::System {
            worker_name: "worker-1".to_string(),
            on_behalf_of: None,
        };
        let record =
            SessionVersionRecord::new(session_id, 1, chrono::Utc::now(), session, Some(actor));
        let summary_record = SessionSummaryRecord::from(&record);
        assert_eq!(summary_record.actor, None);
    }

    #[test]
    fn backward_compat_deserializes_job_id_field() {
        let session_id = crate::SessionId::new();
        let json = serde_json::json!({
            "job_id": session_id.to_string(),
            "status": "ok"
        });
        let resp: KillSessionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.session_id, session_id);
    }

    #[test]
    fn session_serializes_agent_config_mcp_config() {
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "playwright": {
                    "command": "npx",
                    "args": ["@anthropic-ai/mcp-server-playwright"]
                }
            }
        });
        let mut session = make_test_session("mcp test");
        session.agent_config.mcp_config = Some(mcp_config.clone());

        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["agent_config"].get("mcp_config").unwrap(), &mcp_config);

        let deserialized: Session = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.agent_config.mcp_config, Some(mcp_config));
    }

    #[test]
    fn session_mode_round_trips_headless_and_interactive() {
        let headless = SessionMode::Headless;
        let h_json = serde_json::to_value(&headless).unwrap();
        assert_eq!(h_json["type"], "headless");
        // Headless is unit-like; the prompt lives on agent_config.system_prompt.
        assert!(h_json.get("prompt").is_none());
        let parsed: SessionMode = serde_json::from_value(h_json).unwrap();
        assert_eq!(parsed, headless);

        let conv_id = crate::ConversationId::new();
        let interactive = SessionMode::Interactive {
            conversation_id: conv_id.clone(),
            idle_timeout: Timeout::seconds(300),
            greet_user: false,
        };
        let i_json = serde_json::to_value(&interactive).unwrap();
        assert_eq!(i_json["type"], "interactive");
        assert_eq!(i_json["conversation_id"], conv_id.as_ref());
        assert_eq!(i_json["idle_timeout"]["kind"], "seconds");
        assert_eq!(i_json["idle_timeout"]["value"], 300);
        let parsed: SessionMode = serde_json::from_value(i_json).unwrap();
        assert_eq!(parsed, interactive);
    }

    #[test]
    fn session_mode_interactive_serializes_infinite_idle_timeout() {
        let conv_id = crate::ConversationId::new();
        let interactive = SessionMode::Interactive {
            conversation_id: conv_id,
            idle_timeout: Some(Timeout::Infinite),
            greet_user: false,
        };
        let json = serde_json::to_value(&interactive).unwrap();
        assert_eq!(json["idle_timeout"]["kind"], "infinite");
        assert!(json["idle_timeout"].get("value").is_none());
        let parsed: SessionMode = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, interactive);
    }

    // ---------------------------------------------------------------------
    // `AgentConfig.agent_name` is `Option<AgentName>`. The deserializer
    // must accept the validated form (including `null`) and reject
    // malformed historic values, so re-validation happens on every read
    // rather than relying on a one-time backfill.
    // ---------------------------------------------------------------------

    #[test]
    fn agent_config_accepts_none_agent_name_on_deserialize() {
        let json = serde_json::json!({
            "agent_name": null,
            "model": "gpt-4o",
        });
        let cfg: AgentConfig = serde_json::from_value(json).unwrap();
        assert!(cfg.agent_name.is_none());
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn agent_config_accepts_missing_agent_name_on_deserialize() {
        // Field is `#[serde(default)]` so the absence of the key is
        // equivalent to `null`; both produce `None` without error.
        let json = serde_json::json!({"model": "gpt-4o"});
        let cfg: AgentConfig = serde_json::from_value(json).unwrap();
        assert!(cfg.agent_name.is_none());
    }

    #[test]
    fn agent_config_rejects_invalid_agent_name_on_deserialize() {
        // `bad/name` contains `/`, which `AgentName::try_new` rejects.
        // `AgentConfig.agent_name` is typed as `Option<AgentName>`, so
        // malformed strings fail fast at the deserialization boundary
        // instead of slipping through as free `String`s.
        let json = serde_json::json!({"agent_name": "bad/name"});
        let result: Result<AgentConfig, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "expected agent_name 'bad/name' to fail validation, got {result:?}"
        );
    }

    #[test]
    fn agent_config_rejects_whitespace_agent_name_on_deserialize() {
        let json = serde_json::json!({"agent_name": "bad name"});
        let result: Result<AgentConfig, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "expected agent_name 'bad name' to fail validation, got {result:?}"
        );
    }

    #[test]
    fn agent_config_rejects_empty_agent_name_on_deserialize() {
        let json = serde_json::json!({"agent_name": ""});
        let result: Result<AgentConfig, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "expected empty agent_name to fail validation, got {result:?}"
        );
    }

    #[test]
    fn create_session_request_round_trips_interactive_mode() {
        let conv_id = crate::ConversationId::new();
        let request = CreateSessionRequest {
            mode: SessionMode::Interactive {
                conversation_id: conv_id.clone(),
                idle_timeout: None,
                greet_user: false,
            },
            agent_config: AgentSpec::Adhoc {
                system_prompt: "test".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["mode"]["type"], "interactive");
        assert_eq!(json["mode"]["conversation_id"], conv_id.as_ref());

        let deserialized: CreateSessionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(
            deserialized.mode.conversation_id(),
            Some(&conv_id),
            "conversation_id should round-trip through SessionMode::Interactive"
        );
    }

    #[test]
    fn create_session_request_round_trips_headless_mode() {
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Adhoc {
                system_prompt: "do stuff".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["mode"]["type"], "headless");
        // Headless is unit-like — no prompt field on `mode`.
        assert!(json["mode"].get("prompt").is_none());
        assert_eq!(json["agent_config"]["type"], "adhoc");
        assert_eq!(json["agent_config"]["system_prompt"], "do stuff");
        // Optional/empty fields are omitted on the wire.
        assert!(json.get("spawned_from").is_none());
        assert!(json.get("resumed_from").is_none());
    }

    #[test]
    fn agent_spec_named_variant_round_trips() {
        let agent_name = AgentName::try_new("swe").unwrap();
        let spec = AgentSpec::Named {
            name: agent_name.clone(),
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["type"], "named");
        assert_eq!(json["name"], "swe");

        let parsed: AgentSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn agent_spec_adhoc_variant_round_trips() {
        let spec = AgentSpec::Adhoc {
            system_prompt: "do X".to_string(),
            mcp_config: Some(serde_json::json!({"mcpServers": {}})),
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["type"], "adhoc");
        assert_eq!(json["system_prompt"], "do X");
        assert!(json["mcp_config"].is_object());

        let parsed: AgentSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn agent_spec_adhoc_omits_none_mcp_config_on_wire() {
        let spec = AgentSpec::Adhoc {
            system_prompt: "do X".to_string(),
            mcp_config: None,
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["type"], "adhoc");
        assert!(
            json.get("mcp_config").is_none(),
            "None mcp_config should be omitted; got {json:?}"
        );
    }

    #[test]
    fn create_session_request_round_trips_named_agent_spec() {
        let agent_name = AgentName::try_new("swe").unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Named {
                name: agent_name.clone(),
            },
            model: Some("gpt-4o".to_string()),
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["agent_config"]["type"], "named");
        assert_eq!(json["agent_config"]["name"], "swe");
        assert_eq!(json["model"], "gpt-4o");

        let deserialized: CreateSessionRequest = serde_json::from_value(json).unwrap();
        match deserialized.agent_config {
            AgentSpec::Named { name } => assert_eq!(name, agent_name),
            _ => panic!("expected Named variant"),
        }
        assert_eq!(deserialized.model.as_deref(), Some("gpt-4o"));
    }

    fn make_test_worker_context(mode_kind: SessionModeKind) -> WorkerContext {
        WorkerContext::new(
            SessionId::new(),
            mode_kind,
            Vec::new(),
            RelativePath::new("repo").unwrap(),
            Some("claude".to_string()),
            None,
            None,
            HashMap::new(),
            None,
        )
    }

    #[test]
    fn worker_context_serializes_mcp_config() {
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "browser": {"command": "mcp-browser"}
            }
        });
        let mut ctx = make_test_worker_context(SessionModeKind::Headless);
        ctx.mcp_config = Some(mcp_config.clone());

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json.get("mcp_config").unwrap(), &mcp_config);

        let deserialized: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.mcp_config, Some(mcp_config));
    }

    #[test]
    fn worker_context_serializes_interactive_mode_kind() {
        let mut ctx = make_test_worker_context(SessionModeKind::Interactive);
        ctx.idle_timeout = Timeout::seconds(600);

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["mode_kind"], "interactive");
        assert_eq!(json["idle_timeout"]["kind"], "seconds");
        assert_eq!(json["idle_timeout"]["value"], 600);

        let deserialized: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.mode_kind, SessionModeKind::Interactive);
        assert_eq!(deserialized.idle_timeout, Timeout::seconds(600));
    }

    #[test]
    fn worker_context_serializes_infinite_idle_timeout() {
        let mut ctx = make_test_worker_context(SessionModeKind::Interactive);
        ctx.idle_timeout = Some(Timeout::Infinite);

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["idle_timeout"]["kind"], "infinite");

        let deserialized: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.idle_timeout, Some(Timeout::Infinite));
    }

    #[test]
    fn relative_path_accepts_simple_paths() {
        assert!(RelativePath::new("repo").is_ok());
        assert!(RelativePath::new("documents").is_ok());
        assert!(RelativePath::new("a/b/c").is_ok());
        assert!(RelativePath::new("foo/bar").is_ok());
    }

    #[test]
    fn relative_path_rejects_absolute_paths() {
        assert_eq!(
            RelativePath::new("/abs/path").unwrap_err(),
            RelativePathError::Absolute
        );
    }

    #[test]
    fn relative_path_rejects_parent_traversal() {
        assert_eq!(
            RelativePath::new("..").unwrap_err(),
            RelativePathError::ParentTraversal
        );
        assert_eq!(
            RelativePath::new("foo/../bar").unwrap_err(),
            RelativePathError::ParentTraversal
        );
        assert_eq!(
            RelativePath::new("foo/..").unwrap_err(),
            RelativePathError::ParentTraversal
        );
    }

    #[test]
    fn relative_path_rejects_empty() {
        assert_eq!(RelativePath::new("").unwrap_err(), RelativePathError::Empty);
    }

    #[test]
    fn relative_path_serializes_as_string() {
        let path = RelativePath::new("a/b/c").unwrap();
        let json = serde_json::to_value(&path).unwrap();
        assert_eq!(json, serde_json::json!("a/b/c"));
    }

    #[test]
    fn relative_path_deserialize_rejects_absolute() {
        let result: Result<RelativePath, _> = serde_json::from_value(serde_json::json!("/etc"));
        assert!(result.is_err());
    }

    #[test]
    fn relative_path_deserialize_rejects_parent_traversal() {
        let result: Result<RelativePath, _> =
            serde_json::from_value(serde_json::json!("foo/../bar"));
        assert!(result.is_err());
    }

    fn standard_mount_spec(with_build_cache: bool) -> MountSpec {
        use crate::build_cache::{BuildCacheSettings, BuildCacheStorageConfig};

        let repo_target = RelativePath::new("repo").unwrap();
        let docs_target = RelativePath::new("documents").unwrap();
        let bundle = Bundle::GitRepository {
            url: "https://example.test/repo.git".to_string(),
            rev: "main".to_string(),
        };
        let mut mounts = vec![MountItem::Bundle {
            target: repo_target.clone(),
            bundle,
        }];
        if with_build_cache {
            let cache_context = BuildCacheContext {
                storage: BuildCacheStorageConfig::FileSystem {
                    root_dir: "/tmp/cache".to_string(),
                },
                settings: BuildCacheSettings::default(),
            };
            mounts.push(MountItem::BuildCache {
                repo_target: repo_target.clone(),
                service_repo_name: RepoName::try_from("acme/widgets".to_string()).unwrap(),
                context: cache_context,
            });
        }
        mounts.push(MountItem::Documents {
            target: docs_target,
        });
        MountSpec::new(RelativePath::new("repo").unwrap(), mounts)
    }

    #[test]
    fn mount_spec_round_trips_three_item_layout() {
        let spec = standard_mount_spec(true);
        let json = serde_json::to_value(&spec).unwrap();
        let parsed: MountSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
        assert_eq!(parsed.mounts.len(), 3);
    }

    #[test]
    fn mount_spec_round_trips_two_item_layout() {
        let spec = standard_mount_spec(false);
        let json = serde_json::to_value(&spec).unwrap();
        let parsed: MountSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, spec);
        assert_eq!(parsed.mounts.len(), 2);
        // First item is Bundle, second is Documents.
        assert!(matches!(parsed.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(parsed.mounts[1], MountItem::Documents { .. }));
    }

    #[test]
    fn mount_item_unknown_tag_maps_to_unknown_variant() {
        let json = serde_json::json!({
            "working_dir": "repo",
            "mounts": [
                {
                    "type": "bundle",
                    "target": "repo",
                    "bundle": {"type": "none"},
                },
                {
                    "type": "future_secrets_mount",
                    "target": "secrets",
                    "irrelevant": 42
                },
                {"type": "documents", "target": "documents"}
            ]
        });
        let parsed: MountSpec = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.mounts.len(), 3);
        assert!(matches!(parsed.mounts[0], MountItem::Bundle { .. }));
        assert!(matches!(parsed.mounts[1], MountItem::Unknown));
        assert!(matches!(parsed.mounts[2], MountItem::Documents { .. }));
    }

    /// Regression: pre-PR-D `MountItem::Bundle` JSON rows persisted with
    /// `session_id` and `issue_branch_id` must continue to deserialize into
    /// the new fieldless variant. Those fields are silently discarded.
    #[test]
    fn mount_item_bundle_tolerates_legacy_session_metadata_fields() {
        let session_id = SessionId::new();
        let json = serde_json::json!({
            "type": "bundle",
            "target": "repo",
            "bundle": {"type": "none"},
            "session_id": session_id.to_string(),
            "issue_branch_id": "hydra/i-legacy/head",
        });
        let parsed: MountItem = serde_json::from_value(json).unwrap();
        match parsed {
            MountItem::Bundle { target, bundle } => {
                assert_eq!(target.as_path().to_string_lossy(), "repo");
                assert_eq!(bundle, Bundle::None);
            }
            other => panic!("expected MountItem::Bundle, got {other:?}"),
        }
    }

    /// Regression: pre-PR-D `MountItem::BuildCache` JSON rows persisted with
    /// `session_id` must continue to deserialize into the new fieldless
    /// variant. The field is silently discarded.
    #[test]
    fn mount_item_build_cache_tolerates_legacy_session_id_field() {
        use crate::build_cache::{BuildCacheSettings, BuildCacheStorageConfig};

        let session_id = SessionId::new();
        let context = BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: "/tmp/legacy-cache".to_string(),
            },
            settings: BuildCacheSettings::default(),
        };
        let json = serde_json::json!({
            "type": "build_cache",
            "repo_target": "repo",
            "service_repo_name": "acme/widgets",
            "context": context,
            "session_id": session_id.to_string(),
        });
        let parsed: MountItem = serde_json::from_value(json).unwrap();
        match parsed {
            MountItem::BuildCache {
                repo_target,
                service_repo_name,
                context: parsed_context,
            } => {
                assert_eq!(repo_target.as_path().to_string_lossy(), "repo");
                assert_eq!(service_repo_name.to_string(), "acme/widgets");
                assert_eq!(parsed_context, context);
            }
            other => panic!("expected MountItem::BuildCache, got {other:?}"),
        }
    }

    #[test]
    fn worker_context_requires_session_id_for_deserialization() {
        let json = serde_json::json!({
            "resolved_env": {},
        });
        let result: Result<WorkerContext, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "WorkerContext deserialization must fail without session_id",
        );
    }

    #[test]
    fn worker_context_drops_legacy_top_level_fields() {
        let ctx = make_test_worker_context(SessionModeKind::Headless);
        let json = serde_json::to_value(&ctx).unwrap();
        for legacy in [
            "prompt",
            "session",
            "resumed_state",
            "variables",
            "interactive",
            "request_context",
            "build_cache",
        ] {
            assert!(
                json.get(legacy).is_none(),
                "serialized payload must not include legacy {legacy}"
            );
        }
    }

    #[test]
    fn worker_context_round_trips_flat_fields() {
        let session_id = SessionId::new();
        let context = WorkerContext::new(
            session_id.clone(),
            SessionModeKind::Headless,
            Vec::new(),
            RelativePath::new("repo").unwrap(),
            Some("claude".to_string()),
            None,
            None,
            HashMap::from([("KEY".to_string(), "VAL".to_string())]),
            Some("ghp_test".to_string()),
        );
        let json = serde_json::to_value(&context).unwrap();
        assert_eq!(json["session_id"], session_id.as_ref());
        assert_eq!(json["mode_kind"], "headless");
        let parsed: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.session_id, session_id);
        assert_eq!(parsed.mode_kind, SessionModeKind::Headless);
        assert_eq!(
            parsed.resolved_env.get("KEY").map(String::as_str),
            Some("VAL")
        );
        assert_eq!(parsed.github_token.as_deref(), Some("ghp_test"));
    }

    #[test]
    fn session_mode_interactive_greet_user_round_trips() {
        let conv_id = crate::ConversationId::new();
        let mode = SessionMode::Interactive {
            conversation_id: conv_id.clone(),
            idle_timeout: None,
            greet_user: true,
        };
        let json = serde_json::to_value(&mode).unwrap();
        assert_eq!(json["greet_user"], true);
        let parsed: SessionMode = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, mode);
        assert!(parsed.greet_user());
    }

    #[test]
    fn session_mode_interactive_greet_user_default_false() {
        let conv_id = crate::ConversationId::new();
        let json = serde_json::json!({
            "type": "interactive",
            "conversation_id": conv_id.as_ref(),
        });
        let parsed: SessionMode = serde_json::from_value(json).unwrap();
        assert!(!parsed.greet_user());
    }

    #[test]
    fn session_event_user_message_round_trip() {
        let event = SessionEvent::UserMessage {
            content: "hello agent".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"user_message""#));
    }

    #[test]
    fn session_event_assistant_message_round_trip() {
        let event = SessionEvent::AssistantMessage {
            content: "hi there".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"assistant_message""#));
    }

    #[test]
    fn session_event_tool_use_round_trip() {
        let event = SessionEvent::ToolUse {
            tool_name: "browser_navigate".to_string(),
            payload: serde_json::json!({"url": "https://example.test"}),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"tool_use""#));
    }

    #[test]
    fn session_event_suspending_round_trip() {
        let event = SessionEvent::Suspending {
            reason: "idle_timeout".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"suspending""#));
    }

    #[test]
    fn session_event_resumed_native_wire_format() {
        let event = SessionEvent::Resumed {
            from_session_id: SessionId::new(),
            source: ResumeSource::Native,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"resumed""#));
        assert!(json.contains(r#""from_session_id""#));
        assert!(json.contains(r#""source":"native""#));
    }

    #[test]
    fn session_event_resumed_transcript_wire_format() {
        let event = SessionEvent::Resumed {
            from_session_id: SessionId::new(),
            source: ResumeSource::Transcript,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""source":"transcript""#));
    }

    #[test]
    fn session_event_closed_round_trip() {
        let event = SessionEvent::Closed {
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"closed""#));
    }

    #[test]
    fn session_event_unknown_tag_round_trips_as_unknown() {
        let json = r#"{"type":"future_kind","whatever":42}"#;
        let parsed: SessionEvent = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, SessionEvent::Unknown);
    }

    #[test]
    fn session_event_system_event_child_unblocked_round_trip() {
        let child_id = IssueId::new();
        let event = SessionEvent::SystemEvent {
            kind: SystemEventKind::ChildUnblocked {
                child_id: child_id.clone(),
                new_status: crate::api::v1::projects::StatusKey::try_new("in-review").unwrap(),
            },
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"system_event""#));
        assert!(json.contains(r#""kind":"child_unblocked""#));
        assert!(json.contains(child_id.as_ref()));
    }

    #[test]
    fn system_event_kind_child_unblocked_render_is_canonical() {
        let child_id = IssueId::try_from("i-abcdef".to_string()).unwrap();
        let kind = SystemEventKind::ChildUnblocked {
            child_id,
            new_status: crate::api::v1::projects::StatusKey::try_new("complete").unwrap(),
        };
        assert_eq!(
            kind.render(),
            "Child i-abcdef reached status complete; please continue."
        );
    }

    #[test]
    fn session_event_summary_omits_preview_when_absent() {
        let summary = SessionEventSummary::new(0, None);
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("last_event_preview"));
        let parsed: SessionEventSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, summary);
    }
}
