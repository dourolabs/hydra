use crate::{
    BuildCacheContext, ConversationId, IssueId, RepoName, SessionId, VersionNumber,
    actor_ref::ActorRef,
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

/// Settings that only apply when a session is running in interactive mode.
///
/// Present (`Some`) on an interactive session; absent (`None`) on a one-shot session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct InteractiveOptions {
    /// Conversation this session is attached to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Idle timeout in seconds — the worker suspends the session after this
    /// long without a user message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u64>,
    /// When resuming a conversation, the event index to resume from. The worker
    /// sends this in the WorkerConnect handshake so the server only replays
    /// events after this index and includes session state for resumption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_resume_from: Option<usize>,
}

impl InteractiveOptions {
    pub fn new(
        conversation_id: Option<ConversationId>,
        idle_timeout_secs: Option<u64>,
        conversation_resume_from: Option<usize>,
    ) -> Self {
        Self {
            conversation_id,
            idle_timeout_secs,
            conversation_resume_from,
        }
    }
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AgentConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
}

impl AgentConfig {
    pub fn new(
        agent_name: Option<String>,
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
    /// One-shot headless task. The prompt drives the whole run.
    Headless { prompt: String },
    /// Interactive session attached to a conversation.
    Interactive {
        conversation_id: ConversationId,
        idle_timeout_secs: u64,
    },
}

impl SessionMode {
    /// Convenience accessor for the linked conversation (`None` on headless).
    pub fn conversation_id(&self) -> Option<&ConversationId> {
        match self {
            SessionMode::Headless { .. } => None,
            SessionMode::Interactive {
                conversation_id, ..
            } => Some(conversation_id),
        }
    }

    /// Returns the headless prompt, or empty string for interactive mode.
    /// Used to populate the legacy `prompt` wire field during the
    /// Phase-D dual-write transition.
    pub fn prompt_for_legacy_wire(&self) -> &str {
        match self {
            SessionMode::Headless { prompt } => prompt.as_str(),
            SessionMode::Interactive { .. } => "",
        }
    }
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
    /// Server-supplied mount layout. Always populated post-PR-1.
    #[serde(default = "default_mount_spec")]
    pub mount_spec: MountSpec,
    /// Transitional bundle spec, retained until PR-3 routes
    /// `CreateSessionRequest` → `mount_spec` through the resolver. The
    /// in-memory `mount_spec` lowers `ServiceRepository` to a placeholder
    /// `Bundle::None`, so the resolver still relies on this field for
    /// service-repository → git-url translation. Removed in PR-3.
    #[serde(default, skip_serializing_if = "BundleSpec::is_none")]
    pub context: BundleSpec,

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
    #[serde(default = "default_mode")]
    pub mode: SessionMode,

    /// Conversation event index that this resumed session should replay from.
    /// Set by the spawn automation when a previous run on the same
    /// conversation was closed/suspended. Transitional alongside
    /// [`Self::resumed_from`] until PR-4 introduces `SessionStateBlob` and the
    /// worker stops needing an event-index hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_resume_from: Option<usize>,

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
            context: BundleSpec::None,
            image,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            mode,
            conversation_resume_from: None,
            status,
            last_message,
            error,
            deleted,
            creation_time,
            start_time,
            end_time,
            usage: None,
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

fn default_mount_spec() -> MountSpec {
    MountSpec::new(
        RelativePath::new("repo").expect("static `repo` path is valid"),
        Vec::new(),
    )
}

fn default_mode() -> SessionMode {
    SessionMode::Headless {
        prompt: String::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateSessionRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub context: BundleSpec,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    #[serde(default)]
    pub interactive: bool,
}

impl CreateSessionRequest {
    pub fn new(
        prompt: String,
        image: Option<String>,
        context: BundleSpec,
        variables: HashMap<String, String>,
        issue_id: Option<IssueId>,
        conversation_id: Option<ConversationId>,
        interactive: bool,
    ) -> Self {
        Self {
            prompt,
            image,
            context,
            variables,
            issue_id,
            conversation_id,
            interactive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
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
    #[serde(other)]
    Unknown,
}

impl Default for BundleSpec {
    fn default() -> Self {
        Self::None
    }
}

impl BundleSpec {
    pub fn is_none(&self) -> bool {
        matches!(self, BundleSpec::None)
    }
}

impl From<Bundle> for BundleSpec {
    fn from(bundle: Bundle) -> Self {
        match bundle {
            Bundle::None => BundleSpec::None,
            Bundle::GitRepository { url, rev } => BundleSpec::GitRepository { url, rev },
            Bundle::Unknown => BundleSpec::Unknown,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BundleSpecHelper {
    #[serde(rename = "none")]
    None,
    GitRepository {
        url: String,
        rev: String,
    },
    ServiceRepository {
        name: RepoName,
        #[serde(default)]
        rev: Option<String>,
    },
}

impl<'de> Deserialize<'de> for BundleSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<BundleSpecHelper>(value) {
            Ok(BundleSpecHelper::None) => Ok(BundleSpec::None),
            Ok(BundleSpecHelper::GitRepository { url, rev }) => {
                Ok(BundleSpec::GitRepository { url, rev })
            }
            Ok(BundleSpecHelper::ServiceRepository { name, rev }) => {
                Ok(BundleSpec::ServiceRepository { name, rev })
            }
            Err(_) => Ok(BundleSpec::Unknown),
        }
    }
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
}

/// One mount's worth of server-supplied configuration.
///
/// Each variant carries the full set of inputs the corresponding `Mount`
/// constructor needs, including session metadata (`session_id`,
/// `issue_branch_id`) the server already knows at spec construction.
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
        session_id: SessionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issue_branch_id: Option<String>,
    },

    /// Apply / upload the nearest build cache against `repo_target`.
    BuildCache {
        repo_target: RelativePath,
        service_repo_name: RepoName,
        context: BuildCacheContext,
        session_id: SessionId,
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
        session_id: SessionId,
        #[serde(default)]
        issue_branch_id: Option<String>,
    },
    BuildCache {
        repo_target: RelativePath,
        service_repo_name: RepoName,
        context: BuildCacheContext,
        session_id: SessionId,
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
            Ok(MountItemHelper::Bundle {
                target,
                bundle,
                session_id,
                issue_branch_id,
            }) => Ok(MountItem::Bundle {
                target,
                bundle,
                session_id,
                issue_branch_id,
            }),
            Ok(MountItemHelper::BuildCache {
                repo_target,
                service_repo_name,
                context,
                session_id,
            }) => Ok(MountItem::BuildCache {
                repo_target,
                service_repo_name,
                context,
                session_id,
            }),
            Ok(MountItemHelper::Documents { target }) => Ok(MountItem::Documents { target }),
            Err(_) => Ok(MountItem::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkerContext {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
    /// Interactive-only settings. `Some` when the worker should run in
    /// interactive mode; `None` for a one-shot session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<InteractiveOptions>,
    /// Server-supplied mount layout. The server always populates this; the
    /// worker iterates `mount_spec.mounts` to build its per-run mounts.
    pub mount_spec: MountSpec,
}

impl WorkerContext {
    pub fn new(
        prompt: String,
        model: Option<String>,
        variables: HashMap<String, String>,
        mcp_config: Option<McpConfig>,
        interactive: Option<InteractiveOptions>,
        mount_spec: MountSpec,
    ) -> Self {
        Self {
            prompt,
            model,
            variables,
            mcp_config,
            interactive,
            mount_spec,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateSessionResponse {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
}

impl CreateSessionResponse {
    pub fn new(session_id: SessionId) -> Self {
        Self { session_id }
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
        let raw_prompt = match &session.mode {
            SessionMode::Headless { prompt } => prompt.as_str(),
            SessionMode::Interactive { .. } => "",
        };
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

/// Append-only log of model-context events for a session. The transcript the
/// model "sees" is the projection of this log onto `UserMessage` and
/// `AssistantMessage` variants in insertion order.
///
/// Mirrors [`ConversationEvent`](crate::conversations::ConversationEvent) so
/// the same store / cache / SSE plumbing can be reused once Phase B wires the
/// new storage in. See `/designs/sessions-orthogonality-redesign.md` §3.2.
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
    /// first event on a resumed session; carries the predecessor session id.
    Resumed {
        from_session_id: SessionId,
        timestamp: DateTime<Utc>,
    },
    /// Session is closed — no further events will be appended.
    Closed { timestamp: DateTime<Utc> },
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
            | SessionEvent::Closed { timestamp } => Some(*timestamp),
            SessionEvent::Unknown => None,
        }
    }
}

/// Summary of session events for batch fetching — mirrors the
/// `ConversationEventSummary` shape used by the existing conversation read
/// paths so the eventual `get_session_event_summaries` store method can return
/// the same minimal shape per session.
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
            AgentConfig::new(None, Some("claude-3".to_string()), None, None),
            test_mount_spec(),
            Some("worker:latest".to_string()),
            HashMap::from([("KEY".to_string(), "val".to_string())]),
            Some("500m".to_string()),
            Some("1Gi".to_string()),
            Some(vec!["secret".to_string()]),
            SessionMode::Headless {
                prompt: prompt.to_string(),
            },
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
            idle_timeout_secs: 600,
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
        let headless = SessionMode::Headless {
            prompt: "go".to_string(),
        };
        let h_json = serde_json::to_value(&headless).unwrap();
        assert_eq!(h_json["type"], "headless");
        assert_eq!(h_json["prompt"], "go");
        let parsed: SessionMode = serde_json::from_value(h_json).unwrap();
        assert_eq!(parsed, headless);

        let conv_id = crate::ConversationId::new();
        let interactive = SessionMode::Interactive {
            conversation_id: conv_id.clone(),
            idle_timeout_secs: 300,
        };
        let i_json = serde_json::to_value(&interactive).unwrap();
        assert_eq!(i_json["type"], "interactive");
        assert_eq!(i_json["conversation_id"], conv_id.as_ref());
        assert_eq!(i_json["idle_timeout_secs"], 300);
        let parsed: SessionMode = serde_json::from_value(i_json).unwrap();
        assert_eq!(parsed, interactive);
    }

    #[test]
    fn agent_config_round_trips() {
        let cfg = AgentConfig::new(
            Some("agent-x".to_string()),
            Some("gpt-4o".to_string()),
            Some("you are helpful".to_string()),
            Some(serde_json::json!({"servers": {}})),
        );
        let json = serde_json::to_value(&cfg).unwrap();
        let parsed: AgentConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn create_session_request_round_trips_conversation_id() {
        let conv_id = crate::ConversationId::new();
        let request = CreateSessionRequest::new(
            "prompt".to_string(),
            None,
            BundleSpec::None,
            HashMap::new(),
            None,
            Some(conv_id.clone()),
            true,
        );

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(
            json.get("conversation_id").and_then(|v| v.as_str()),
            Some(conv_id.as_ref())
        );

        let deserialized: CreateSessionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.conversation_id, Some(conv_id));
    }

    #[test]
    fn create_session_request_omits_conversation_id_when_none() {
        let request = CreateSessionRequest::new(
            "prompt".to_string(),
            None,
            BundleSpec::None,
            HashMap::new(),
            None,
            None,
            false,
        );

        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("conversation_id").is_none());
    }

    #[test]
    fn worker_context_serializes_mcp_config() {
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "browser": {"command": "mcp-browser"}
            }
        });
        let context = WorkerContext::new(
            "test prompt".to_string(),
            None,
            HashMap::new(),
            Some(mcp_config.clone()),
            None,
            standard_mount_spec(false),
        );

        let json = serde_json::to_value(&context).unwrap();
        assert_eq!(json.get("mcp_config").unwrap(), &mcp_config);

        let deserialized: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.mcp_config, Some(mcp_config));
    }

    #[test]
    fn worker_context_serializes_interactive_options() {
        let opts = InteractiveOptions::new(None, Some(600), Some(42));
        let context = WorkerContext::new(
            "test prompt".to_string(),
            None,
            HashMap::new(),
            None,
            Some(opts.clone()),
            standard_mount_spec(false),
        );

        let json = serde_json::to_value(&context).unwrap();
        let interactive = json.get("interactive").expect("interactive present");
        assert_eq!(interactive.get("idle_timeout_secs").unwrap(), 600);
        assert_eq!(interactive.get("conversation_resume_from").unwrap(), 42);

        let deserialized: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.interactive, Some(opts));
    }

    #[test]
    fn worker_context_omits_interactive_when_none() {
        let context = WorkerContext::new(
            "test prompt".to_string(),
            None,
            HashMap::new(),
            None,
            None,
            standard_mount_spec(false),
        );

        let json = serde_json::to_value(&context).unwrap();
        assert!(json.get("interactive").is_none());
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

    #[test]
    fn relative_path_round_trips() {
        let path = RelativePath::new("repo").unwrap();
        let json = serde_json::to_value(&path).unwrap();
        let parsed: RelativePath = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, path);
    }

    fn standard_mount_spec(with_build_cache: bool) -> MountSpec {
        use crate::build_cache::{BuildCacheSettings, BuildCacheStorageConfig};

        let repo_target = RelativePath::new("repo").unwrap();
        let docs_target = RelativePath::new("documents").unwrap();
        let session_id = SessionId::new();
        let bundle = Bundle::GitRepository {
            url: "https://example.test/repo.git".to_string(),
            rev: "main".to_string(),
        };
        let mut mounts = vec![MountItem::Bundle {
            target: repo_target.clone(),
            bundle,
            session_id: session_id.clone(),
            issue_branch_id: Some("hydra/i-abcd/head".to_string()),
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
                session_id: session_id.clone(),
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
        let session_id = SessionId::new();
        let json = serde_json::json!({
            "working_dir": "repo",
            "mounts": [
                {
                    "type": "bundle",
                    "target": "repo",
                    "bundle": {"type": "none"},
                    "session_id": session_id.to_string(),
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

    #[test]
    fn worker_context_requires_mount_spec_for_deserialization() {
        let json = serde_json::json!({
            "prompt": "hello",
            "variables": {},
        });
        let result: Result<WorkerContext, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "WorkerContext deserialization must fail without mount_spec",
        );
    }

    #[test]
    fn worker_context_rejects_legacy_request_context_and_build_cache() {
        let spec = standard_mount_spec(true);
        let context = WorkerContext::new(
            "test prompt".to_string(),
            None,
            HashMap::new(),
            None,
            None,
            spec.clone(),
        );
        let json = serde_json::to_value(&context).unwrap();
        assert!(
            json.get("request_context").is_none(),
            "serialized payload must not include legacy request_context"
        );
        assert!(
            json.get("build_cache").is_none(),
            "serialized payload must not include legacy build_cache"
        );
    }

    #[test]
    fn worker_context_serializes_mount_spec_when_present() {
        let spec = standard_mount_spec(true);
        let context = WorkerContext::new(
            "test prompt".to_string(),
            None,
            HashMap::new(),
            None,
            None,
            spec.clone(),
        );
        let json = serde_json::to_value(&context).unwrap();
        assert!(json.get("mount_spec").is_some());
        let parsed: WorkerContext = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.mount_spec, spec);
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
    fn session_event_resumed_round_trip() {
        let event = SessionEvent::Resumed {
            from_session_id: SessionId::new(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert!(json.contains(r#""type":"resumed""#));
        assert!(json.contains(r#""from_session_id""#));
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
    fn session_event_summary_round_trip() {
        let summary = SessionEventSummary::new(7, Some("User: hi".to_string()));
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: SessionEventSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(summary, parsed);
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
