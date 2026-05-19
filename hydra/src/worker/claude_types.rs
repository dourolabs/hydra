//! Claude-native I/O vocabulary for the worker. See
//! `designs/worker-model-commands-refactor.md` §2 for the design.
//!
//! These types are **Claude-native**: they never appear on `ModelSelector`'s
//! generic surface (PR 2's concern) — they exist as a stable home so PR 2 can
//! move them into `claude.rs` without churning call sites.

use std::path::PathBuf;

/// Resume input for a Claude run. Both variants ultimately become
/// `claude --resume <UUID>`; the `TranscriptFile` variant is sugar for the
/// wrapper-side "install transcript at Claude's expected path, then resume by
/// UUID" sequence.
#[derive(Debug, Clone)]
pub enum ClaudeResume {
    SessionId(String),
    TranscriptFile(PathBuf),
}

/// One user message destined for Claude's stdin (stream-json input).
#[derive(Debug, Clone)]
pub struct ClaudeUserMessage {
    pub content: String,
}

/// One event emitted by Claude on its stdout (stream-json output), parsed
/// into a typed shape. Variants and fields mirror what `claude_formatter.rs`
/// already extracts from the wire — see `StreamFormatter::handle_assistant`
/// and `StreamFormatter::handle_user`.
#[derive(Debug, Clone)]
pub enum ClaudeEvent {
    /// `assistant`-typed stream-json line. Variant carries the text content
    /// of any `text` blocks in the message (matches the value
    /// `StreamFormatter::last_assistant_text` ends up holding).
    Assistant { text: String },
    /// Per-turn token usage block from an `assistant` line's
    /// `message.usage` field. Field names match the wire keys.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    },
    /// `system` init line carrying Claude's session UUID.
    SystemInit { session_id: String },
    /// Catch-all for stream-json lines the variants above don't cover —
    /// `user` (tool_result) lines, `result` lines, etc. The raw JSON value
    /// is kept so consumers that want an audit trail can record it.
    Raw { value: serde_json::Value },
}
