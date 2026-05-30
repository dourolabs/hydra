//! Generic model dispatch. `ModelSelector` is an enum (not a trait) that
//! routes batch / interactive runs to the per-model wrappers and translates
//! the generic `WorkerInputMessage` / `WorkerEvent` vocabulary into each
//! model's native I/O types inside the `match` arms.
//!
//! See `designs/worker-model-commands-refactor.md` §3 and §6, and
//! `designs/sessions-worker-run-interface.md` §3.1 / §3.2 for the
//! `drive_headless` / `drive_interactive` dispatch surface that owns the
//! three WS phases.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use anyhow::{anyhow, Result};
use futures::{Sink, Stream};
use hydra_common::api::v1::{
    conversations::{ServerMessage, SessionStatePayload, WorkerMessage},
    sessions::{ResumeSource, SessionEvent},
};
use hydra_common::SessionId;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite;

use crate::worker::claude::{Claude, ClaudeEvent, ClaudeResume, ClaudeUserMessage};
use crate::worker::codex::Codex;
use crate::worker::relay_adapter::{spawn_relay_pump, PumpExit, ReconnectFn, RelayAdapter};
use crate::worker::report::{
    MaterializeError, NativeResume, RunReport, SessionResume, TokenUsage, WorkerEvent,
    WorkerInputMessage,
};
use crate::worker::socket::WorkerSocket;

/// Brief non-blocking drain duration for inbound `ServerMessage::EndSession`
/// at the tail of `drive_headless`. Headless does not concurrently read the
/// WS while the model runs (the per-wrapper `run` call is a single await),
/// so any `EndSession` issued during that window sits buffered. A short
/// poll picks it up; the tradeoff is the cost of a tiny extra wait on the
/// natural-exit path. The unified cleanup still runs unconditionally.
const HEADLESS_END_SESSION_DRAIN: Duration = Duration::from_millis(100);

/// Routes a worker invocation to either the Claude or Codex per-model wrapper.
///
/// Constructed once per worker via [`ModelSelector::from_context`]; the public
/// surface is [`Self::drive_headless`] / [`Self::drive_interactive`], which
/// own all three WS phases (negotiate, first-message, model-stream pump).
pub enum ModelSelector {
    Claude(Claude),
    Codex(Codex),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Kind {
    Claude,
    Codex,
}

impl ModelSelector {
    /// Build a `ModelSelector` from the per-worker context. Decides between
    /// `Claude` and `Codex` based on the model name (see [`Self::decide_kind`])
    /// and constructs the chosen wrapper, propagating any setup error.
    pub async fn from_context(
        model: &Option<String>,
        working_dir: PathBuf,
        home_dir: PathBuf,
        env: HashMap<String, String>,
        mcp_config_json: Option<&str>,
        idle_timeout: Duration,
    ) -> Result<Self> {
        let kind = Self::decide_kind(model.as_deref());
        match kind {
            Kind::Claude => Ok(Self::Claude(
                Claude::new(
                    model.clone(),
                    working_dir,
                    home_dir,
                    env,
                    mcp_config_json,
                    idle_timeout,
                )
                .await?,
            )),
            Kind::Codex => Ok(Self::Codex(
                Codex::new(model.clone(), working_dir, home_dir, env, mcp_config_json).await?,
            )),
        }
    }

    /// Drive a complete headless (non-interactive) run on `ws`. Owns Phase 1
    /// (context negotiation), Phase 2 (first-message), and the per-wrapper
    /// model invocation, plus the unified end-of-session cleanup
    /// (`SessionStateUpload` → `Closed` event → optional `EndSessionAck`).
    /// Returns the resulting [`RunReport`].
    pub async fn drive_headless<S>(&mut self, mut ws: WorkerSocket<S>) -> Result<RunReport>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin,
    {
        let (resume, primer) = self.phase1_negotiate(&mut ws).await?;
        let (agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let prompt = concat_first_message(&primer, &agent_prompt, &user_message);
        let report = self.run_with_native(&prompt, resume).await?;
        // Headless doesn't concurrently read the WS during `run_with_native`,
        // so any `EndSession` issued during the model run is buffered. Drain
        // it now (non-blocking-ish) before the unified cleanup so we can
        // include `EndSessionAck`. The cleanup runs unconditionally.
        let end_session_requested = drain_end_session_headless(&mut ws).await;
        send_unified_cleanup(&mut ws, &report, end_session_requested).await;
        Ok(report)
    }

    /// Drive a complete interactive run on `ws`. Owns Phase 1 / Phase 2,
    /// then hands the socket to the [`crate::worker::relay_adapter`] pump for
    /// Phase 3, and finally drives the unified end-of-session cleanup
    /// (`SessionStateUpload` → `Closed` event → optional `EndSessionAck`).
    /// Returns the resulting [`RunReport`]. `session_id` and `reconnect`
    /// are forwarded to the pump so it can reopen the WS on a mid-session
    /// drop while the model is still running.
    pub async fn drive_interactive<S>(
        &mut self,
        mut ws: WorkerSocket<S>,
        session_id: SessionId,
        reconnect: ReconnectFn<S>,
    ) -> Result<RunReport>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin
            + Send
            + 'static,
    {
        let (resume, primer) = self.phase1_negotiate(&mut ws).await?;
        let (agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let prompt = concat_first_message(&primer, &agent_prompt, &user_message);
        let RelayAdapter {
            input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, reconnect);
        let report = self
            .run_interactive_with_native(input_rx, output_tx, &prompt, resume)
            .await?;
        let PumpExit {
            ws,
            end_session_requested,
        } = pump.await.unwrap_or(PumpExit {
            ws: None,
            end_session_requested: false,
        });
        if let Some(mut ws) = ws {
            send_unified_cleanup(&mut ws, &report, end_session_requested).await;
        }
        Ok(report)
    }

    /// Phase 1 — context negotiation. Sends `Fresh`, awaits `ResumeContext`,
    /// attempts native materialization, and on failure asks for the prior
    /// session's transcript as primer text. Returns the resolved
    /// [`NativeResume`] (if any) and the primer-event list (may be empty).
    async fn phase1_negotiate<S>(
        &mut self,
        ws: &mut WorkerSocket<S>,
    ) -> Result<(Option<NativeResume>, Vec<SessionEvent>)>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin,
    {
        ws.send(WorkerMessage::Fresh).await?;
        let resume_ctx = ws
            .recv()
            .await?
            .ok_or_else(|| anyhow!("ws closed before ResumeContext"))?;
        let (resume_blob, prior_session_id) = match resume_ctx {
            ServerMessage::ResumeContext {
                resume_blob,
                prior_session_id,
            } => (resume_blob, prior_session_id),
            other => return Err(anyhow!("expected ResumeContext, got {other:?}")),
        };

        // Try native materialization first; on Err, fall back to transcript replay.
        let (native, need_transcript) = match resume_blob {
            Some(bytes) => match self.try_materialize_resume(&bytes).await {
                Ok(native) => (Some(native), false),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        prior_session_id = ?prior_session_id,
                        "native resume materialization failed; falling back to transcript replay",
                    );
                    (None, prior_session_id.is_some())
                }
            },
            None => (None, prior_session_id.is_some()),
        };

        let primer = emit_resumed_and_collect_primer(
            ws,
            native.is_some(),
            prior_session_id,
            need_transcript,
        )
        .await?;

        Ok((native, primer))
    }

    /// Run one batch turn. The Codex arm currently has no resume support;
    /// `resume` is logged and dropped if supplied.
    async fn run_with_native(
        &mut self,
        prompt: &str,
        resume: Option<NativeResume>,
    ) -> Result<RunReport> {
        match self {
            Self::Claude(c) => {
                let claude_resume = resume.and_then(|r| match r {
                    NativeResume::Claude(c) => Some(c),
                    _ => None,
                });
                c.run(prompt, claude_resume).await
            }
            Self::Codex(c) => {
                if resume.is_some() {
                    tracing::debug!("ModelSelector: dropping `resume` for Codex (out of scope)");
                }
                c.run(prompt).await
            }
        }
    }

    /// Per-wrapper materialization of a resume blob. Returns
    /// `Err(NotImplemented)` for wrappers that don't support resume; the
    /// caller falls back to transcript-primer replay either way.
    async fn try_materialize_resume(
        &self,
        state_bytes: &[u8],
    ) -> std::result::Result<NativeResume, MaterializeError> {
        match self {
            Self::Claude(c) => c.try_materialize(state_bytes),
            Self::Codex(c) => c.try_materialize(state_bytes),
        }
    }

    /// Run one interactive session. The Codex arm always returns `Err`
    /// (`codex` does not have a long-lived stdin/stdout shape); the caller is
    /// expected to kind-check before opening the relay WebSocket so this is a
    /// belt-and-suspenders guard.
    async fn run_interactive_with_native(
        &mut self,
        input_rx: mpsc::Receiver<WorkerInputMessage>,
        output_tx: mpsc::Sender<WorkerEvent>,
        prompt: &str,
        resume: Option<NativeResume>,
    ) -> Result<RunReport> {
        match self {
            Self::Claude(c) => {
                let claude_resume = resume.and_then(|r| match r {
                    NativeResume::Claude(c) => Some(c),
                    _ => None,
                });
                let (claude_in_tx, claude_in_rx) = mpsc::channel::<ClaudeUserMessage>(32);
                let (claude_out_tx, claude_out_rx) = mpsc::channel::<ClaudeEvent>(32);
                let in_pump = spawn_input_translator(input_rx, claude_in_tx);
                let out_pump = spawn_output_translator(claude_out_rx, output_tx);
                let report = c
                    .run_interactive(claude_in_rx, claude_out_tx, prompt, claude_resume)
                    .await;
                let _ = in_pump.await;
                let _ = out_pump.await;
                report
            }
            Self::Codex(_) => Err(anyhow!("model does not support interactive mode")),
        }
    }

    /// Side-effect-free probe over the model name: returns `true` if the name
    /// resolves to a wrapper that supports interactive (long-lived
    /// stdin/stdout) runs, `false` otherwise. Used by `worker_run` to
    /// short-circuit Codex+interactive before constructing the wrapper, which
    /// would otherwise perform per-worker setup (e.g. `codex login`, writing
    /// `~/.codex/config.toml`, creating the output tempdir).
    pub(crate) fn supports_interactive(name: Option<&str>) -> bool {
        matches!(Self::decide_kind(name), Kind::Claude)
    }

    /// Decide which kind of model wrapper this name maps to. Name-based
    /// (per design §6); matches either an exact bare family name or one of
    /// the `<family>-...` prefix forms:
    ///
    /// * `claude` / `haiku` / `sonnet` / `opus` (bare) or `claude-` /
    ///   `haiku-` / `sonnet-` / `opus-` (prefix) → [`Kind::Claude`]
    /// * `gpt-` / `o1` / `o3` / `o4` / `codex-` → [`Kind::Codex`]
    /// * Everything else (and `None`) → [`Kind::Codex`] with a `warn!` log so
    ///   misroutes are spottable.
    fn decide_kind(name: Option<&str>) -> Kind {
        let Some(raw) = name else {
            return Kind::Codex;
        };
        let lc = raw.to_ascii_lowercase();
        let claude_exact = ["claude", "haiku", "sonnet", "opus"];
        let claude_prefixes = ["claude-", "haiku-", "sonnet-", "opus-"];
        let codex_prefixes = ["gpt-", "o1", "o3", "o4", "codex-"];
        if claude_exact.iter().any(|n| lc == *n) {
            return Kind::Claude;
        }
        if claude_prefixes.iter().any(|p| lc.starts_with(p)) {
            return Kind::Claude;
        }
        if codex_prefixes.iter().any(|p| lc.starts_with(p)) {
            return Kind::Codex;
        }
        tracing::warn!(model = %raw, "ModelSelector: model name unrecognized, defaulting to Codex");
        Kind::Codex
    }
}

/// Phase 1 — resume-side effects. Per design §1.4 / §6 the worker emits
/// `SessionEvent::Resumed` exactly once on its session log whenever it
/// actually restored from a prior session. Both the native-materialization
/// path and the transcript-replay fallback are observable; the `source`
/// field on the event distinguishes them. The emit is gated on
/// `prior_session_id.is_some()` so a truly fresh session (no predecessor)
/// stays silent. When the new worker must replay the prior transcript as
/// primer text, this also performs the `RequestTranscript`/`Transcript`
/// round-trip and returns the events.
///
/// Extracted from [`ModelSelector::phase1_negotiate`] so unit tests can
/// drive it without standing up a real [`Claude`]/[`Codex`] wrapper. The
/// per-wrapper `try_materialize_resume` decision is summarised here by the
/// boolean `native_present` parameter.
async fn emit_resumed_and_collect_primer<S>(
    ws: &mut WorkerSocket<S>,
    native_present: bool,
    prior_session_id: Option<SessionId>,
    need_transcript: bool,
) -> Result<Vec<SessionEvent>>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    let (primer, emit_source) = match (need_transcript, prior_session_id.clone()) {
        (true, Some(prior)) => {
            ws.send(WorkerMessage::RequestTranscript {
                prior_session_id: prior,
            })
            .await?;
            let resp = ws
                .recv()
                .await?
                .ok_or_else(|| anyhow!("ws closed before Transcript"))?;
            let events = match resp {
                ServerMessage::Transcript { events } => events,
                other => return Err(anyhow!("expected Transcript, got {other:?}")),
            };
            (events, Some(ResumeSource::Transcript))
        }
        _ if native_present && prior_session_id.is_some() => {
            (Vec::new(), Some(ResumeSource::Native))
        }
        _ => (Vec::new(), None),
    };

    if let (Some(from), Some(source)) = (prior_session_id, emit_source) {
        ws.send(WorkerMessage::Event {
            event: SessionEvent::Resumed {
                from_session_id: from,
                source,
                timestamp: chrono::Utc::now(),
            },
        })
        .await?;
    }

    Ok(primer)
}

/// Build a `SessionStatePayload` from a [`RunReport`] for the unified
/// cleanup `SessionStateUpload`. Returns `None` if the report didn't
/// observe an on-disk session-state file or didn't extract a model
/// session id — in either case there is nothing to upload that the
/// resumer can use.
fn build_session_state_payload(report: &RunReport) -> Option<SessionStatePayload> {
    let state_ref = report.session_state.as_ref()?;
    let session_id = report.model_session_id.clone()?;
    let transcript = std::fs::read(&state_ref.local_path).ok();
    Some(SessionStatePayload::V1 {
        session_id,
        transcript,
    })
}

/// Unified end-of-session cleanup. Sent on BOTH the natural-exit and the
/// `EndSession`-driven paths so the resumer sees `session_state` regardless
/// of how the worker shut down. Order matters: the `SessionStateUpload`
/// arrives before the `Closed` event so the server commits state before
/// marking the log closed. `EndSessionAck` is the very last frame so the
/// server-side waiter (PR-2) can know its termination request was honored.
async fn send_unified_cleanup<S>(
    ws: &mut WorkerSocket<S>,
    report: &RunReport,
    end_session_requested: bool,
) where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    if let Some(payload) = build_session_state_payload(report) {
        match serde_json::to_vec(&payload) {
            Ok(data) => {
                if let Err(err) = ws.send(WorkerMessage::SessionStateUpload { data }).await {
                    tracing::warn!(error = %err, "cleanup: failed to send SessionStateUpload");
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "cleanup: failed to serialize SessionStatePayload");
            }
        }
    }
    if let Err(err) = ws
        .send(WorkerMessage::Event {
            event: SessionEvent::Closed {
                timestamp: chrono::Utc::now(),
            },
        })
        .await
    {
        tracing::warn!(error = %err, "cleanup: failed to send Closed event");
    }
    if end_session_requested {
        if let Err(err) = ws.send(WorkerMessage::EndSessionAck).await {
            tracing::warn!(error = %err, "cleanup: failed to send EndSessionAck");
        }
    }
}

/// Best-effort headless drain of an inbound `ServerMessage::EndSession`.
/// Returns `true` iff `EndSession` was observed. Per the design's "simple
/// shape" for headless: no concurrent watcher during the model run; just
/// peek after it returns.
async fn drain_end_session_headless<S>(ws: &mut WorkerSocket<S>) -> bool
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    match tokio::time::timeout(HEADLESS_END_SESSION_DRAIN, ws.recv()).await {
        Ok(Ok(Some(ServerMessage::EndSession))) => true,
        Ok(Ok(Some(other))) => {
            tracing::warn!(?other, "headless drain: dropping unexpected ServerMessage");
            false
        }
        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => false,
    }
}

/// Phase 2 — send `Ready`, await `FirstMessage`, return its contents.
async fn phase2_ready<S>(ws: &mut WorkerSocket<S>) -> Result<(String, String)>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    ws.send(WorkerMessage::Ready).await?;
    let msg = ws
        .recv()
        .await?
        .ok_or_else(|| anyhow!("ws closed before FirstMessage"))?;
    match msg {
        ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        } => Ok((agent_prompt, user_message)),
        other => Err(anyhow!("expected FirstMessage, got {other:?}")),
    }
}

/// Concatenate the optional primer (from a transcript-replay fallback),
/// the agent prompt, and the user message into one model-input string,
/// collapsing empty pieces.
fn concat_first_message(
    primer_events: &[SessionEvent],
    agent_prompt: &str,
    user_message: &str,
) -> String {
    let primer = primer_to_text(primer_events);
    let base = match (agent_prompt, user_message) {
        ("", "") => String::new(),
        ("", u) => u.to_string(),
        (p, "") => p.to_string(),
        (p, u) => format!("{p}\n\n{u}"),
    };
    if primer.is_empty() {
        base
    } else if base.is_empty() {
        primer
    } else {
        format!("{primer}\n\n{base}")
    }
}

fn primer_to_text(events: &[SessionEvent]) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            SessionEvent::UserMessage { content, .. } => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("User: ");
                out.push_str(content);
            }
            SessionEvent::AssistantMessage { content, .. } => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("Assistant: ");
                out.push_str(content);
            }
            _ => {}
        }
    }
    out
}

/// Translate a generic `SessionResume` into Claude's native shape.
#[allow(dead_code)]
fn claude_resume_from_generic(r: SessionResume) -> ClaudeResume {
    match r {
        SessionResume::BySessionId(id) => ClaudeResume::SessionId(id),
        SessionResume::ByTranscriptFile(path) => ClaudeResume::TranscriptFile(path),
    }
}

/// Pump `WorkerInputMessage` → `ClaudeUserMessage`. Stops when the input
/// channel closes.
fn spawn_input_translator(
    mut g: mpsc::Receiver<WorkerInputMessage>,
    c: mpsc::Sender<ClaudeUserMessage>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(msg) = g.recv().await {
            if c.send(ClaudeUserMessage {
                content: msg.content,
            })
            .await
            .is_err()
            {
                break;
            }
        }
    })
}

/// Pump `ClaudeEvent` → `WorkerEvent`. Stops when the input channel closes.
fn spawn_output_translator(
    mut c: mpsc::Receiver<ClaudeEvent>,
    g: mpsc::Sender<WorkerEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = c.recv().await {
            let translated = translate_claude_event(event);
            if g.send(translated).await.is_err() {
                break;
            }
        }
    })
}

/// Per-event translation between Claude-native and generic event shapes.
/// Pulled out so unit tests can exercise it directly without spinning up the
/// translator task.
fn translate_claude_event(event: ClaudeEvent) -> WorkerEvent {
    match event {
        ClaudeEvent::Assistant { text } => WorkerEvent::AssistantText { text },
        ClaudeEvent::Usage {
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        } => WorkerEvent::Usage {
            usage: TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
            },
        },
        ClaudeEvent::SystemInit { session_id } => WorkerEvent::SessionInit {
            model_session_id: session_id,
        },
        ClaudeEvent::ToolUse { tool_name, payload } => WorkerEvent::ToolUse { tool_name, payload },
        ClaudeEvent::Raw { value } => WorkerEvent::Raw { value },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn decide_kind_claude_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("claude-3-5-sonnet")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_opus_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("opus-4-via-bedrock")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_haiku_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("haiku-4-5-20251001")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_sonnet_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("sonnet-4-6")), Kind::Claude);
    }

    #[test]
    fn decide_kind_bare_claude_family_names() {
        assert_eq!(ModelSelector::decide_kind(Some("claude")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("opus")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("sonnet")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("haiku")), Kind::Claude);
    }

    #[test]
    fn decide_kind_bare_claude_family_names_case_insensitive() {
        assert_eq!(ModelSelector::decide_kind(Some("Opus")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("SONNET")), Kind::Claude);
    }

    #[test]
    fn decide_kind_gpt_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("gpt-4o")), Kind::Codex);
    }

    #[test]
    fn decide_kind_codex_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("codex-cli")), Kind::Codex);
    }

    #[test]
    fn supports_interactive_true_for_claude_names() {
        assert!(ModelSelector::supports_interactive(Some(
            "claude-3-5-sonnet"
        )));
        assert!(ModelSelector::supports_interactive(Some("opus-4")));
        assert!(ModelSelector::supports_interactive(Some("sonnet-4-6")));
        assert!(ModelSelector::supports_interactive(Some(
            "haiku-4-5-20251001"
        )));
    }

    #[test]
    fn supports_interactive_true_for_bare_claude_family_names() {
        assert!(ModelSelector::supports_interactive(Some("claude")));
        assert!(ModelSelector::supports_interactive(Some("opus")));
        assert!(ModelSelector::supports_interactive(Some("sonnet")));
        assert!(ModelSelector::supports_interactive(Some("haiku")));
    }

    #[test]
    fn supports_interactive_false_for_codex_names() {
        assert!(!ModelSelector::supports_interactive(Some("gpt-4o")));
        assert!(!ModelSelector::supports_interactive(Some("codex-cli")));
        assert!(!ModelSelector::supports_interactive(Some("o1")));
        assert!(!ModelSelector::supports_interactive(Some("o3")));
        assert!(!ModelSelector::supports_interactive(Some("o4")));
    }

    #[test]
    fn supports_interactive_false_for_unknown_and_none() {
        assert!(!ModelSelector::supports_interactive(Some("unknown")));
        assert!(!ModelSelector::supports_interactive(None));
    }

    #[test]
    fn claude_resume_from_generic_by_session_id() {
        let r = claude_resume_from_generic(SessionResume::BySessionId("x".to_string()));
        match r {
            ClaudeResume::SessionId(id) => assert_eq!(id, "x"),
            other => panic!("expected SessionId, got {other:?}"),
        }
    }

    #[test]
    fn claude_resume_from_generic_by_transcript_file() {
        let p = PathBuf::from("/tmp/transcript.jsonl");
        let r = claude_resume_from_generic(SessionResume::ByTranscriptFile(p.clone()));
        match r {
            ClaudeResume::TranscriptFile(path) => assert_eq!(path, p),
            other => panic!("expected TranscriptFile, got {other:?}"),
        }
    }

    #[test]
    fn translate_assistant_event_to_assistant_text() {
        let event = ClaudeEvent::Assistant {
            text: "hi".to_string(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::AssistantText { text } => assert_eq!(text, "hi"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn translate_usage_event_to_usage() {
        let event = ClaudeEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::Usage { usage } => {
                assert_eq!(usage.input_tokens, 1);
                assert_eq!(usage.output_tokens, 2);
                assert_eq!(usage.cache_read_input_tokens, 3);
                assert_eq!(usage.cache_creation_input_tokens, 4);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn translate_system_init_to_session_init() {
        let event = ClaudeEvent::SystemInit {
            session_id: "uuid".to_string(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::SessionInit { model_session_id } => assert_eq!(model_session_id, "uuid"),
            other => panic!("expected SessionInit, got {other:?}"),
        }
    }

    #[test]
    fn translate_raw_event_to_raw() {
        let value = serde_json::json!({"type": "weird", "x": 1});
        let event = ClaudeEvent::Raw {
            value: value.clone(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::Raw { value: v } => assert_eq!(v, value),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn translate_tool_use_event_to_tool_use() {
        let payload = serde_json::json!({"command": "ls", "description": "list"});
        let event = ClaudeEvent::ToolUse {
            tool_name: "Bash".to_string(),
            payload: payload.clone(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::ToolUse {
                tool_name,
                payload: p,
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(p, payload);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn concat_first_message_collapses_empty_pieces() {
        assert_eq!(concat_first_message(&[], "", ""), "");
        assert_eq!(concat_first_message(&[], "prompt", ""), "prompt");
        assert_eq!(concat_first_message(&[], "", "user"), "user");
        assert_eq!(
            concat_first_message(&[], "prompt", "user"),
            "prompt\n\nuser"
        );
    }

    #[tokio::test]
    async fn spawn_output_translator_translates_event_stream() {
        let (claude_tx, claude_rx) = mpsc::channel::<ClaudeEvent>(8);
        let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerEvent>(8);
        let handle = spawn_output_translator(claude_rx, worker_tx);

        claude_tx
            .send(ClaudeEvent::Assistant {
                text: "hi".to_string(),
            })
            .await
            .unwrap();
        claude_tx
            .send(ClaudeEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
            .await
            .unwrap();
        claude_tx
            .send(ClaudeEvent::SystemInit {
                session_id: "x".to_string(),
            })
            .await
            .unwrap();
        let raw_val = serde_json::json!({"type": "weird"});
        claude_tx
            .send(ClaudeEvent::Raw {
                value: raw_val.clone(),
            })
            .await
            .unwrap();
        drop(claude_tx);
        handle.await.unwrap();

        let mut got = Vec::new();
        while let Some(e) = worker_rx.recv().await {
            got.push(e);
        }
        assert_eq!(got.len(), 4);
        assert!(matches!(got[0], WorkerEvent::AssistantText { ref text } if text == "hi"));
        assert!(matches!(
            got[1],
            WorkerEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    ..
                }
            }
        ));
        assert!(
            matches!(got[2], WorkerEvent::SessionInit { ref model_session_id } if model_session_id == "x")
        );
        assert!(matches!(got[3], WorkerEvent::Raw { ref value } if value == &raw_val));
    }

    #[tokio::test]
    async fn spawn_input_translator_passes_through_content() {
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerInputMessage>(8);
        let (claude_tx, mut claude_rx) = mpsc::channel::<ClaudeUserMessage>(8);
        let handle = spawn_input_translator(worker_rx, claude_tx);

        worker_tx
            .send(WorkerInputMessage {
                content: "hello".to_string(),
            })
            .await
            .unwrap();
        drop(worker_tx);
        handle.await.unwrap();

        let msg = claude_rx.recv().await.unwrap();
        assert_eq!(msg.content, "hello");
        assert!(claude_rx.recv().await.is_none());
    }

    // ---------------------------------------------------------------------
    // emit_resumed_and_collect_primer — covers the dual-path Resumed emit
    // (per design §1.4 / §6) end-to-end against a real `WorkerSocket`
    // wired to an in-memory duplex. Replaces the "fake-worker" shape that
    // existed before [[i-eaawhkqo]]: that earlier harness silently dropped
    // the transcript-replay Resumed emit, which was the bug surfaced in
    // [[i-mnjxuojq]] / [[i-hnrdnsfb]].
    // ---------------------------------------------------------------------

    use crate::worker::socket::WorkerSocket;
    use futures::channel::mpsc as futures_mpsc;
    use futures::{Sink, StreamExt};
    use hydra_common::api::v1::conversations::{ServerMessage, WorkerMessage};
    use hydra_common::api::v1::sessions::ResumeSource;
    use hydra_common::SessionId;
    use tokio_tungstenite::tungstenite;

    type WsFrame = std::result::Result<tungstenite::Message, tungstenite::Error>;
    type WsRx = futures_mpsc::UnboundedReceiver<WsFrame>;
    type WsTx = futures_mpsc::UnboundedSender<WsFrame>;

    /// Sink+Stream duplex over `futures::channel::mpsc`, used to back a
    /// `WorkerSocket` in tests.
    struct TestStream {
        rx: WsRx,
        tx: WsTx,
    }

    impl futures::Stream for TestStream {
        type Item = std::result::Result<tungstenite::Message, tungstenite::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::pin::Pin::new(&mut self.rx).poll_next(cx)
        }
    }

    impl Sink<tungstenite::Message> for TestStream {
        type Error = tungstenite::Error;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn start_send(
            self: std::pin::Pin<&mut Self>,
            item: tungstenite::Message,
        ) -> std::result::Result<(), Self::Error> {
            self.tx
                .unbounded_send(Ok(item))
                .map_err(|_| tungstenite::Error::ConnectionClosed)
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            self.tx.close_channel();
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// Builds a `WorkerSocket` and the raw server-side half of the
    /// duplex; the server half is used to push canned `ServerMessage`s and
    /// inspect outgoing `WorkerMessage`s.
    fn paired() -> (WorkerSocket<TestStream>, WsRx, WsTx) {
        let (w_tx, s_rx) = futures_mpsc::unbounded();
        let (s_tx, w_rx) = futures_mpsc::unbounded();
        let worker = WorkerSocket::new(TestStream { rx: w_rx, tx: w_tx });
        (worker, s_rx, s_tx)
    }

    async fn next_worker_msg(s_rx: &mut WsRx) -> WorkerMessage {
        let frame = s_rx
            .next()
            .await
            .expect("worker sent no frame")
            .expect("frame error");
        let text = match frame {
            tungstenite::Message::Text(t) => t,
            other => panic!("expected text frame, got {other:?}"),
        };
        serde_json::from_str(&text).expect("parse WorkerMessage")
    }

    async fn push_server_msg(s_tx: &WsTx, msg: &ServerMessage) {
        let json = serde_json::to_string(msg).unwrap();
        s_tx.unbounded_send(Ok(tungstenite::Message::Text(json)))
            .unwrap();
    }

    #[tokio::test]
    async fn emit_resumed_native_path_sends_resumed_with_native_source() {
        let (mut worker, mut s_rx, _s_tx) = paired();
        let prior = SessionId::new();

        let primer = emit_resumed_and_collect_primer(&mut worker, true, Some(prior.clone()), false)
            .await
            .unwrap();

        assert!(
            primer.is_empty(),
            "native path must not return primer events"
        );
        match next_worker_msg(&mut s_rx).await {
            WorkerMessage::Event {
                event:
                    SessionEvent::Resumed {
                        from_session_id,
                        source,
                        ..
                    },
            } => {
                assert_eq!(from_session_id, prior);
                assert_eq!(source, ResumeSource::Native);
            }
            other => panic!("expected Resumed{{Native}}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_resumed_transcript_path_requests_transcript_then_emits_resumed_transcript() {
        let (mut worker, mut s_rx, s_tx) = paired();
        let prior = SessionId::new();
        let canned = vec![SessionEvent::UserMessage {
            content: "msg1".to_string(),
            timestamp: chrono::Utc::now(),
        }];

        // The helper sends RequestTranscript first; reply with canned events.
        let primer_handle = {
            let prior = prior.clone();
            tokio::spawn(async move {
                emit_resumed_and_collect_primer(&mut worker, false, Some(prior), true)
                    .await
                    .unwrap()
            })
        };

        // 1. Worker sends RequestTranscript.
        match next_worker_msg(&mut s_rx).await {
            WorkerMessage::RequestTranscript { prior_session_id } => {
                assert_eq!(prior_session_id, prior);
            }
            other => panic!("expected RequestTranscript, got {other:?}"),
        }
        // 2. Server replies with the transcript.
        push_server_msg(
            &s_tx,
            &ServerMessage::Transcript {
                events: canned.clone(),
            },
        )
        .await;
        // 3. Worker emits Resumed{Transcript}.
        match next_worker_msg(&mut s_rx).await {
            WorkerMessage::Event {
                event:
                    SessionEvent::Resumed {
                        from_session_id,
                        source,
                        ..
                    },
            } => {
                assert_eq!(from_session_id, prior);
                assert_eq!(source, ResumeSource::Transcript);
            }
            other => panic!("expected Resumed{{Transcript}}, got {other:?}"),
        }
        let primer = primer_handle.await.unwrap();
        assert_eq!(primer, canned, "primer must be the transcript replay");
    }

    #[tokio::test]
    async fn emit_resumed_fresh_session_emits_nothing() {
        let (mut worker, mut s_rx, _s_tx) = paired();

        // No predecessor session: helper must not send anything.
        let primer = emit_resumed_and_collect_primer(&mut worker, false, None, false)
            .await
            .unwrap();
        assert!(primer.is_empty());
        // Close the sender side so `next` returns None instead of hanging.
        drop(_s_tx);
        drop(worker);
        assert!(s_rx.next().await.is_none(), "no frames must be sent");
    }

    mod unified_cleanup {
        use super::*;
        use crate::worker::report::{RunReport, SessionStateFormat, SessionStateRef, TokenUsage};
        use futures::SinkExt;
        use std::io::Write;
        use tokio_tungstenite::tungstenite;

        type WsFrame = std::result::Result<tungstenite::Message, tungstenite::Error>;
        type WsSender = futures::channel::mpsc::UnboundedSender<WsFrame>;
        type WsReceiver = futures::channel::mpsc::UnboundedReceiver<WsFrame>;

        struct TestStream {
            rx: futures::channel::mpsc::UnboundedReceiver<WsFrame>,
            tx: futures::channel::mpsc::UnboundedSender<WsFrame>,
        }
        impl futures::Stream for TestStream {
            type Item = WsFrame;
            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Self::Item>> {
                std::pin::Pin::new(&mut self.rx).poll_next(cx)
            }
        }
        impl futures::Sink<tungstenite::Message> for TestStream {
            type Error = tungstenite::Error;
            fn poll_ready(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }
            fn start_send(
                self: std::pin::Pin<&mut Self>,
                item: tungstenite::Message,
            ) -> std::result::Result<(), Self::Error> {
                self.tx
                    .unbounded_send(Ok(item))
                    .map_err(|_| tungstenite::Error::ConnectionClosed)
            }
            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }
            fn poll_close(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
                self.tx.close_channel();
                std::task::Poll::Ready(Ok(()))
            }
        }

        fn duplex() -> (WorkerSocket<TestStream>, WsSender, WsReceiver) {
            let (server_tx, worker_rx) = futures::channel::mpsc::unbounded::<WsFrame>();
            let (worker_tx, server_rx) = futures::channel::mpsc::unbounded::<WsFrame>();
            let ws = WorkerSocket::new(TestStream {
                rx: worker_rx,
                tx: worker_tx,
            });
            (ws, server_tx, server_rx)
        }

        async fn collect_worker_msgs(server_rx: &mut WsReceiver) -> Vec<WorkerMessage> {
            use futures::StreamExt;
            let mut out = Vec::new();
            while let Some(Ok(frame)) = server_rx.next().await {
                if let tungstenite::Message::Text(text) = frame {
                    if let Ok(msg) = serde_json::from_str::<WorkerMessage>(&text) {
                        out.push(msg);
                    }
                }
            }
            out
        }

        fn report_with_state(transcript_bytes: &[u8]) -> (tempfile::NamedTempFile, RunReport) {
            let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
            tmp.write_all(transcript_bytes).expect("write");
            let local_path = tmp.path().to_path_buf();
            let report = RunReport {
                last_message: "done".to_string(),
                usage: TokenUsage::default(),
                model_session_id: Some("sess-123".to_string()),
                session_state: Some(SessionStateRef {
                    local_path,
                    format: SessionStateFormat::ClaudeJsonl,
                }),
            };
            (tmp, report)
        }

        #[tokio::test]
        async fn build_session_state_payload_skips_when_no_session_id() {
            let (_tmp, mut report) = report_with_state(b"{}\n");
            report.model_session_id = None;
            assert!(build_session_state_payload(&report).is_none());
        }

        #[tokio::test]
        async fn build_session_state_payload_skips_when_no_state_ref() {
            let mut report = RunReport {
                last_message: String::new(),
                usage: TokenUsage::default(),
                model_session_id: Some("sess".to_string()),
                session_state: None,
            };
            report.session_state = None;
            assert!(build_session_state_payload(&report).is_none());
        }

        #[tokio::test]
        async fn build_session_state_payload_reads_transcript_bytes() {
            let (_tmp, report) = report_with_state(b"{\"hello\":true}\n");
            let payload = build_session_state_payload(&report).expect("payload");
            match payload {
                SessionStatePayload::V1 {
                    session_id,
                    transcript,
                } => {
                    assert_eq!(session_id, "sess-123");
                    assert_eq!(transcript.as_deref(), Some(&b"{\"hello\":true}\n"[..]));
                }
            }
        }

        #[tokio::test]
        async fn build_session_state_payload_missing_file_yields_none_transcript() {
            // local_path that doesn't exist — the payload still carries a
            // session_id so the resumer at least sees it; transcript is None
            // and the resumer falls back to the primer path.
            let report = RunReport {
                last_message: String::new(),
                usage: TokenUsage::default(),
                model_session_id: Some("sess-x".to_string()),
                session_state: Some(SessionStateRef {
                    local_path: std::path::PathBuf::from("/nonexistent/transcript.jsonl"),
                    format: SessionStateFormat::ClaudeJsonl,
                }),
            };
            let payload = build_session_state_payload(&report).expect("payload");
            match payload {
                SessionStatePayload::V1 {
                    session_id,
                    transcript,
                } => {
                    assert_eq!(session_id, "sess-x");
                    assert!(transcript.is_none());
                }
            }
        }

        #[tokio::test]
        async fn unified_cleanup_natural_exit_sends_upload_then_closed_no_ack() {
            let (mut ws, _server_tx, mut server_rx) = duplex();
            let (_tmp, report) = report_with_state(b"hello\n");
            send_unified_cleanup(&mut ws, &report, false).await;
            drop(ws);
            let frames = collect_worker_msgs(&mut server_rx).await;
            // Expect exactly: SessionStateUpload, Closed event. No ack.
            assert_eq!(frames.len(), 2, "got frames {frames:?}");
            match &frames[0] {
                WorkerMessage::SessionStateUpload { data } => {
                    let payload: SessionStatePayload = serde_json::from_slice(data).unwrap();
                    match payload {
                        SessionStatePayload::V1 {
                            session_id,
                            transcript,
                        } => {
                            assert_eq!(session_id, "sess-123");
                            assert_eq!(transcript.as_deref(), Some(&b"hello\n"[..]));
                        }
                    }
                }
                other => panic!("expected SessionStateUpload, got {other:?}"),
            }
            match &frames[1] {
                WorkerMessage::Event {
                    event: SessionEvent::Closed { .. },
                } => {}
                other => panic!("expected Closed event, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn unified_cleanup_end_session_appends_ack_after_closed() {
            let (mut ws, _server_tx, mut server_rx) = duplex();
            let (_tmp, report) = report_with_state(b"hello\n");
            send_unified_cleanup(&mut ws, &report, true).await;
            drop(ws);
            let frames = collect_worker_msgs(&mut server_rx).await;
            // Expect: SessionStateUpload, Closed, EndSessionAck.
            assert_eq!(frames.len(), 3, "got frames {frames:?}");
            assert!(matches!(
                frames[0],
                WorkerMessage::SessionStateUpload { .. }
            ));
            assert!(matches!(
                frames[1],
                WorkerMessage::Event {
                    event: SessionEvent::Closed { .. }
                }
            ));
            assert!(matches!(frames[2], WorkerMessage::EndSessionAck));
        }

        #[tokio::test]
        async fn unified_cleanup_skips_upload_when_no_session_state() {
            // No on-disk state ref and no session id — cleanup still sends
            // the `Closed` event so the server-side log is properly capped.
            let (mut ws, _server_tx, mut server_rx) = duplex();
            let report = RunReport {
                last_message: String::new(),
                usage: TokenUsage::default(),
                model_session_id: None,
                session_state: None,
            };
            send_unified_cleanup(&mut ws, &report, false).await;
            drop(ws);
            let frames = collect_worker_msgs(&mut server_rx).await;
            assert_eq!(frames.len(), 1);
            assert!(matches!(
                frames[0],
                WorkerMessage::Event {
                    event: SessionEvent::Closed { .. }
                }
            ));
        }

        #[tokio::test]
        async fn drain_end_session_headless_returns_true_when_present() {
            let (mut ws, mut server_tx, _server_rx) = duplex();
            // Push EndSession before the drain is called so it's immediately
            // in the buffer.
            let json = serde_json::to_string(&ServerMessage::EndSession).unwrap();
            server_tx
                .send(Ok(tungstenite::Message::Text(json)))
                .await
                .unwrap();
            assert!(drain_end_session_headless(&mut ws).await);
        }

        #[tokio::test]
        async fn drain_end_session_headless_returns_false_when_absent() {
            let (mut ws, _server_tx, _server_rx) = duplex();
            // Nothing buffered, timeout returns false.
            assert!(!drain_end_session_headless(&mut ws).await);
        }
    }
}
