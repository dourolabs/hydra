//! End-of-run report types and the generic, model-agnostic I/O vocabulary used
//! by the worker. See `designs/worker-model-commands-refactor.md` §3–4 for the
//! design that motivates this module.
//!
//! `RunReport` is the value `ModelSelector::run` / `run_interactive` return.
//! The generic types (`SessionResume`, `WorkerInputMessage`, `WorkerEvent`)
//! live here as the model-agnostic surface.

use std::path::PathBuf;

pub use hydra_common::sessions::TokenUsage;

/// Result of one batch or interactive worker run, returned by
/// `ModelSelector::run` / `ModelSelector::run_interactive`.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// The final assistant text emitted by the model. Used to populate
    /// `SessionStatusUpdate::Complete { last_message }`.
    pub last_message: String,
    /// Aggregated token usage for the run.
    pub usage: TokenUsage,
    /// The model's internal session id (e.g. Claude's session UUID or Codex's
    /// thread id), if the wrapper observed one.
    pub model_session_id: Option<String>,
    /// Pointer to the on-disk session-state file (Claude transcript or Codex
    /// JSONL), if such a file exists for this run.
    pub session_state: Option<SessionStateRef>,
}

/// Pointer to a session-state file on disk.
#[derive(Debug, Clone)]
pub struct SessionStateRef {
    pub local_path: PathBuf,
    pub format: SessionStateFormat,
}

/// On-disk format of a session-state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStateFormat {
    /// Claude's `~/.claude/projects/<encoded-cwd>/<UUID>.jsonl` transcript.
    ClaudeJsonl,
    /// Codex's `codex exec --json` stdout stream captured as JSONL.
    CodexJsonl,
}

/// Generic resume input, used by `ModelSelector::run` / `run_interactive` (PR
/// 2). Defined in PR 1 so PR 2 can wire it without re-defining.
#[derive(Debug, Clone)]
pub enum SessionResume {
    /// Resume by the provider's internal session id.
    BySessionId(String),
    /// Resume from a transcript file on disk. Each model variant translates
    /// this into its native resume flow.
    ByTranscriptFile(PathBuf),
}

/// One user message destined for the model. Generic over the model — each
/// `ModelSelector::run_interactive` arm translates this into its model-native
/// input message shape.
#[derive(Debug, Clone)]
pub struct WorkerInputMessage {
    pub content: String,
}

/// One event emitted by the model on its output stream, normalized to a
/// model-agnostic shape.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    AssistantText { text: String },
    Usage { usage: TokenUsage },
    SessionInit { model_session_id: String },
    Raw { value: serde_json::Value },
}
