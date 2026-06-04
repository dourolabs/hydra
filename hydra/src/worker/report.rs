//! End-of-run report types and the generic, model-agnostic I/O vocabulary
//! used by the worker.
//!
//! `RunReport` is the value `ModelSelector::drive_headless` /
//! `drive_interactive` return — a flat struct (not an enum) so callers
//! read `last_message`, `usage`, `model_session_id`, and `session_state`
//! directly without matching on a per-model shape; `None` on the
//! `Option` fields means the underlying signal was unavailable, not that
//! the run failed. The generic types (`SessionResume`,
//! `WorkerInputMessage`, `WorkerEvent`) live here as the model-agnostic
//! surface that [`crate::worker::ModelSelector`] translates into each
//! wrapper's native vocabulary inside its `match` arms.

use std::path::PathBuf;

use hydra_common::api::v1::relay::SessionStatePayload;
pub use hydra_common::sessions::TokenUsage;

use crate::util::format_thousands;
use crate::worker::claude::ClaudeResume;
use crate::worker::codex::CodexResume;

/// Result of one batch or interactive worker run, returned by
/// `ModelSelector::drive_headless` / `ModelSelector::drive_interactive`.
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

impl RunReport {
    /// Build a `SessionStatePayload` for the unified cleanup
    /// `SessionStateUpload`. Returns `None` if the report didn't observe an
    /// on-disk session-state file or didn't extract a model session id — in
    /// either case there is nothing to upload that the resumer can use.
    pub(crate) fn session_state_payload(&self) -> Option<SessionStatePayload> {
        let state_ref = self.session_state.as_ref()?;
        let session_id = self.model_session_id.clone()?;
        let transcript = std::fs::read(&state_ref.local_path).ok();
        Some(SessionStatePayload::V1 {
            session_id,
            transcript,
        })
    }

    /// Print a human-readable summary of this report via `println!`.
    pub(crate) fn log(&self) {
        for line in self.format_lines() {
            println!("{line}");
        }
    }

    /// Format this report as a vector of human-readable lines, one per
    /// summary row (tokens, model session id, session state).
    pub(crate) fn format_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(3);
        let total = self
            .usage
            .input_tokens
            .saturating_add(self.usage.output_tokens)
            .saturating_add(self.usage.cache_read_input_tokens)
            .saturating_add(self.usage.cache_creation_input_tokens);
        lines.push(format!(
            "  tokens: input={} output={} cache_read={} cache_create={} total={}",
            format_thousands(self.usage.input_tokens),
            format_thousands(self.usage.output_tokens),
            format_thousands(self.usage.cache_read_input_tokens),
            format_thousands(self.usage.cache_creation_input_tokens),
            format_thousands(total),
        ));
        match &self.model_session_id {
            Some(id) => lines.push(format!("  model_session_id: {id}")),
            None => lines.push("  model_session_id: <none>".to_string()),
        }
        match &self.session_state {
            Some(s) => lines.push(format!(
                "  session_state: {} ({:?})",
                s.local_path.display(),
                s.format,
            )),
            None => lines.push("  session_state: <none>".to_string()),
        }
        lines
    }
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

/// Generic resume input.
#[derive(Debug, Clone)]
pub enum SessionResume {
    /// Resume by the provider's internal session id.
    BySessionId(String),
    /// Resume from a transcript file on disk. Each model variant translates
    /// this into its native resume flow.
    ByTranscriptFile(PathBuf),
}

/// One user message destined for the model. Generic over the model — each
/// `ModelSelector::drive_interactive` arm translates this into its
/// model-native input message shape.
#[derive(Debug, Clone)]
pub struct WorkerInputMessage {
    pub content: String,
}

/// One event emitted by the model on its output stream, normalized to a
/// model-agnostic shape.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    AssistantText {
        text: String,
    },
    Usage {
        usage: TokenUsage,
    },
    SessionInit {
        model_session_id: String,
    },
    /// A tool invocation observed in the model's output stream. The
    /// `tool_name` is the model's tool identifier (e.g. `"Bash"`); the
    /// `payload` is the raw input passed to the tool, as JSON.
    ToolUse {
        tool_name: String,
        payload: serde_json::Value,
    },
    Raw {
        value: serde_json::Value,
    },
}

/// Per-wrapper resume handle, opaque to the dispatcher. Each variant carries
/// the wrapper-native shape so the dispatcher never needs to translate.
///
/// Returned by `try_materialize` on each per-model wrapper; consumed by the
/// same wrapper's `run` / `run_interactive` to drive a resume.
///
/// This is the eventual replacement for the still-present [`SessionResume`]
/// — once the wrappers' native vocabulary is wired all the way through the
/// dispatch layer, the generic shape goes away and only `NativeResume`
/// remains.
#[derive(Debug, Clone)]
pub enum NativeResume {
    Claude(ClaudeResume),
    Codex(CodexResume),
}

/// Failure modes for [`crate::worker::claude::Claude::try_materialize`] and
/// the parallel methods on other per-model wrappers.
///
/// The dispatcher only inspects which variant fired for logging; behavior on
/// `Err` is identical (fall back to the transcript-replay primer path).
#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    /// The state bytes are not in this wrapper's expected format. Treat as a
    /// cross-model handoff (or older / future encoding) and fall back to
    /// transcript replay.
    #[error("state bytes are not in this wrapper's expected format")]
    WrongFormat,
    /// The bytes parsed as this wrapper's payload but did not carry an
    /// on-disk transcript. The bare session id alone is not enough for the
    /// wrapper to resume on a fresh worker, so the dispatcher must fall back
    /// to transcript replay.
    #[error("payload parsed but carried no transcript to materialize")]
    MissingTranscript,
    /// The bytes parsed but writing the wrapper's on-disk artifact failed.
    #[error("failed to write resume artifact: {0}")]
    IoError(#[from] std::io::Error),
    /// This wrapper does not (yet) implement native resume. Codex returns
    /// this today; will become a real implementation when Codex resume lands.
    #[error("native resume is not implemented for this wrapper")]
    NotImplemented,
}
