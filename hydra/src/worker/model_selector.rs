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
    relay::{ServerMessage, SessionStatePayload, WorkerMessage},
    sessions::{ResumeSource, SessionEvent},
};
use hydra_common::SessionId;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite;

use crate::worker::claude::{
    translate_claude_event, Claude, ClaudeEvent, ClaudeResume, ClaudeUserMessage,
};
use crate::worker::codex::Codex;
use crate::worker::relay_adapter::{
    spawn_relay_pump, worker_event_to_session_event, PumpExit, ReconnectFn, RelayAdapter,
};
use crate::worker::report::{
    MaterializeError, NativeResume, RunReport, SessionResume, WorkerEvent, WorkerInputMessage,
};
use crate::worker::socket::WorkerSocket;

/// Brief non-blocking drain duration for inbound `ServerMessage::EndSession`
/// at the tail of `drive_headless`. Headless does not concurrently read the
/// WS while the model runs (the per-wrapper `run` call is a single await),
/// so any `EndSession` issued during that window sits buffered. A short
/// poll picks it up; the tradeoff is the cost of a tiny extra wait on the
/// natural-exit path. The unified cleanup still runs unconditionally.
const HEADLESS_END_SESSION_DRAIN: Duration = Duration::from_millis(100);

/// Outer bound for `forwarder.await` in `drive_headless`. The wrapper's
/// stdout-pump shutdown chain (`PROCESS_GROUP_GRACE_PERIOD` of 5s →
/// `kill_process_group` → `PIPE_READ_TIMEOUT` of 10s, see
/// `worker::claude`) is finite, so under normal conditions the wrapper's
/// `Sender<WorkerEvent>` drops well inside this window and the forwarder
/// hands the WS back via the natural-close branch. This bound only fires
/// when an orphan task still holds a `Sender<WorkerEvent>` clone after
/// `Claude::run` returned — `event_rx.recv().await` would otherwise block
/// indefinitely and wedge `drive_headless`. On timeout we abort the
/// forwarder, log a warning, and treat the result as `None` so the
/// `if let Some(mut ws) = ws_opt` cleanup branch is skipped and the
/// server-side disconnect fallback caps the events log.
const HEADLESS_FORWARDER_DRAIN: Duration = Duration::from_secs(30);

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
    ///
    /// While the model runs, per-step `WorkerEvent`s the per-wrapper `run`
    /// emits are forwarded onto the WS as `WorkerMessage::Event` frames by a
    /// small dedicated forwarder task — the headless analogue of
    /// [`Self::drive_interactive`]'s Phase-3 [`spawn_relay_pump`]. This is
    /// what populates the server-side events log for headless sessions.
    pub async fn drive_headless<S>(&mut self, mut ws: WorkerSocket<S>) -> Result<RunReport>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin
            + Send
            + 'static,
    {
        let (resume, primer, transcript_agent_prompt) = self.phase1_negotiate(&mut ws).await?;
        let (first_msg_agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let agent_prompt = transcript_agent_prompt.unwrap_or(first_msg_agent_prompt);
        let prompt = concat_first_message(&primer, &agent_prompt, &user_message);

        // Phase 3 — hand the WS to a forwarder task that drains
        // `WorkerEvent`s emitted by the wrapper and writes them on the WS
        // as `WorkerMessage::Event` frames. The wrapper drops its sender
        // when its stdout pump finishes, which closes the channel and
        // lets the forwarder hand the WS back for the unified cleanup.
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(32);
        let forwarder = spawn_headless_event_forwarder(ws, event_rx);
        let run_result = self.run_with_native(&prompt, resume, Some(event_tx)).await;
        let ws_opt = await_headless_forwarder(forwarder, HEADLESS_FORWARDER_DRAIN).await;
        let report = run_result?;

        // Headless doesn't concurrently read the WS during `run_with_native`,
        // so any `EndSession` issued during the model run is buffered. Drain
        // it now (non-blocking-ish) before the unified cleanup so we can
        // include `EndSessionAck`. The cleanup runs unconditionally when we
        // still hold a WS — if the forwarder lost it (send failure mid-run)
        // there is nothing left to write on, and the server-side disconnect
        // fallback caps the log.
        if let Some(mut ws) = ws_opt {
            let end_session_requested = drain_end_session_headless(&mut ws).await;
            send_unified_cleanup(&mut ws, &report, end_session_requested).await;
            if let Err(err) = ws.close().await {
                tracing::warn!(error = %err, "cleanup: failed to send WS Close frame");
            }
        }
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
        let (resume, primer, transcript_agent_prompt) = self.phase1_negotiate(&mut ws).await?;
        let (first_msg_agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let agent_prompt = transcript_agent_prompt.unwrap_or(first_msg_agent_prompt);
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
            if let Err(err) = ws.close().await {
                tracing::warn!(error = %err, "cleanup: failed to send WS Close frame");
            }
        }
        Ok(report)
    }

    /// Phase 1 — context negotiation. Sends `Fresh`, awaits `ResumeContext`,
    /// attempts native materialization, and on failure asks for the prior
    /// session's transcript as primer text. Returns the resolved
    /// [`NativeResume`] (if any), the primer-event list (may be empty), and
    /// the transcript-resume agent prompt (Some only on the
    /// transcript-replay branch; the resumed-session `FirstMessage`
    /// blanks the prompt, so the caller uses this value instead when set).
    async fn phase1_negotiate<S>(
        &mut self,
        ws: &mut WorkerSocket<S>,
    ) -> Result<(Option<NativeResume>, Vec<SessionEvent>, Option<String>)>
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

        let (primer, transcript_agent_prompt) = emit_resumed_and_collect_primer(
            ws,
            native.is_some(),
            prior_session_id,
            need_transcript,
        )
        .await?;

        Ok((native, primer, transcript_agent_prompt))
    }

    /// Run one batch turn. The Codex arm currently has no resume support;
    /// `resume` is logged and dropped if supplied. If `event_tx` is `Some`,
    /// it is handed to the per-wrapper `run` so per-step `WorkerEvent`s the
    /// wrapper produces are forwarded to the caller as they arrive.
    async fn run_with_native(
        &mut self,
        prompt: &str,
        resume: Option<NativeResume>,
        event_tx: Option<mpsc::Sender<WorkerEvent>>,
    ) -> Result<RunReport> {
        match self {
            Self::Claude(c) => {
                let claude_resume = resume.and_then(|r| match r {
                    NativeResume::Claude(c) => Some(c),
                    _ => None,
                });
                c.run(prompt, claude_resume, event_tx).await
            }
            Self::Codex(c) => {
                if resume.is_some() {
                    tracing::debug!("ModelSelector: dropping `resume` for Codex (out of scope)");
                }
                c.run(prompt, event_tx).await
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
) -> Result<(Vec<SessionEvent>, Option<String>)>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    let (primer, transcript_agent_prompt, emit_source) =
        match (need_transcript, prior_session_id.clone()) {
            (true, Some(prior)) => {
                ws.send(WorkerMessage::RequestTranscript {
                    prior_session_id: prior,
                })
                .await?;
                let resp = ws
                    .recv()
                    .await?
                    .ok_or_else(|| anyhow!("ws closed before Transcript"))?;
                let (events, agent_prompt) = match resp {
                    ServerMessage::Transcript {
                        events,
                        agent_prompt,
                    } => (events, agent_prompt),
                    other => return Err(anyhow!("expected Transcript, got {other:?}")),
                };
                (events, agent_prompt, Some(ResumeSource::Transcript))
            }
            _ if native_present && prior_session_id.is_some() => {
                (Vec::new(), None, Some(ResumeSource::Native))
            }
            _ => (Vec::new(), None, None),
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

    Ok((primer, transcript_agent_prompt))
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
/// marking the log closed. `EndSessionAck` is the very last `WorkerMessage`
/// frame so the server-side waiter (PR-2) can know its termination request
/// was honored. After this helper returns, the `drive_*` callers drive a
/// `WorkerSocket::close` so the server's `pump_phase3` observes the
/// WebSocket Close-frame arm (`info!("WebSocket closed by worker")`) rather
/// than tungstenite reporting an abrupt TCP teardown as a protocol error
/// (`error!("WebSocket error in Phase 3")`).
///
/// Note: only the natural-exit paths reach this helper. If
/// `run_with_native` / `run_interactive_with_native` returns `Err`, the
/// `drive_*` callers propagate via `?` and cleanup (including the Close
/// frame) is skipped entirely. That is intentional — on a failed run
/// `report.session_state` is `None` so the upload would no-op anyway,
/// and the server-side disconnect fallback caps the log when the WS
/// drops.
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

/// Spawn the headless Phase-3 forwarder. Owns the [`WorkerSocket`] while the
/// model runs and drains `event_rx`, translating each [`WorkerEvent`] into a
/// `WorkerMessage::Event` via [`worker_event_to_session_event`]. Returns the
/// open `WorkerSocket` when the channel closes (per-wrapper `run` finished —
/// its event sender was dropped) so [`ModelSelector::drive_headless`] can
/// drive the unified end-of-session cleanup on it. Returns `None` if a WS
/// send error during forwarding made the socket unusable.
///
/// This is the headless analogue of the Phase-3 pump in
/// [`crate::worker::relay_adapter::spawn_relay_pump`] but trimmed: headless
/// has no inbound `WorkerInputMessage` stream and no reconnect loop, just a
/// uni-directional event-out fanout.
fn spawn_headless_event_forwarder<S>(
    ws: WorkerSocket<S>,
    event_rx: mpsc::Receiver<WorkerEvent>,
) -> JoinHandle<Option<WorkerSocket<S>>>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin
        + Send
        + 'static,
{
    tokio::spawn(async move {
        let mut ws = ws;
        let mut event_rx = event_rx;
        while let Some(event) = event_rx.recv().await {
            let Some(api_event) = worker_event_to_session_event(event) else {
                continue;
            };
            if let Err(err) = ws.send(WorkerMessage::Event { event: api_event }).await {
                tracing::warn!(
                    error = %err,
                    "headless event forwarder: WS send failed; abandoning WS",
                );
                return None;
            }
        }
        Some(ws)
    })
}

/// Await the headless event forwarder, bounded by `drain`. On natural close,
/// returns whatever the forwarder produced (`Some(ws)` for a healthy hand-back
/// or `None` if a mid-stream WS send error already abandoned the socket). On
/// timeout — the per-wrapper shutdown failed to drop every
/// `Sender<WorkerEvent>` clone, so the forwarder is wedged on
/// `event_rx.recv().await` — abort the task, log a warning, and return `None`
/// so the caller skips the cleanup write path. See
/// [`HEADLESS_FORWARDER_DRAIN`] for the rationale.
async fn await_headless_forwarder<S>(
    forwarder: JoinHandle<Option<WorkerSocket<S>>>,
    drain: Duration,
) -> Option<WorkerSocket<S>>
where
    S: Send + 'static,
{
    let abort_handle = forwarder.abort_handle();
    match tokio::time::timeout(drain, forwarder).await {
        Ok(Ok(opt)) => opt,
        Ok(Err(err)) => {
            tracing::warn!(
                error = %err,
                "headless event forwarder task ended abnormally; abandoning WS",
            );
            None
        }
        Err(_) => {
            abort_handle.abort();
            tracing::warn!(
                drain_secs = drain.as_secs(),
                "headless event forwarder did not drain within timeout; aborting (orphan task is still holding a WorkerEvent sender)",
            );
            None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::report::TokenUsage;
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

    /// Resume-path invariant: when `phase1_negotiate` returns a
    /// transcript-borne agent prompt (Some), `drive_*` selects it over
    /// the (blanked) `FirstMessage.agent_prompt` via `unwrap_or`, so the
    /// resulting concatenation feeds the model the system prompt that
    /// `handle_ready` blanked for the resumed-session arm. Without the
    /// new field, the model would see only the primer + user message.
    #[test]
    fn resume_path_concat_uses_transcript_prompt_when_first_message_blanked() {
        // Returned by helpers so clippy doesn't fold the unwrap_or away.
        fn transcript_prompt() -> Option<String> {
            Some("you are helpful".to_string())
        }
        fn first_msg_prompt() -> String {
            String::new()
        }
        let agent_prompt = transcript_prompt().unwrap_or_else(first_msg_prompt);
        let primer = vec![SessionEvent::UserMessage {
            content: "earlier".to_string(),
            timestamp: chrono::Utc::now(),
        }];
        let out = concat_first_message(&primer, &agent_prompt, "new turn");
        assert_eq!(out, "User: earlier\n\nyou are helpful\n\nnew turn");
    }

    /// Fresh-path invariant: with no transcript-borne prompt, the
    /// `FirstMessage.agent_prompt` flows through unchanged — `unwrap_or`
    /// must not clobber the fresh-session prompt with an empty string.
    #[test]
    fn fresh_path_concat_uses_first_message_prompt_when_no_transcript_prompt() {
        fn transcript_prompt() -> Option<String> {
            None
        }
        fn first_msg_prompt() -> String {
            "fresh prompt".to_string()
        }
        let agent_prompt = transcript_prompt().unwrap_or_else(first_msg_prompt);
        let out = concat_first_message(&[], &agent_prompt, "hello");
        assert_eq!(out, "fresh prompt\n\nhello");
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
    use hydra_common::api::v1::relay::{ServerMessage, WorkerMessage};
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

        let (primer, transcript_agent_prompt) =
            emit_resumed_and_collect_primer(&mut worker, true, Some(prior.clone()), false)
                .await
                .unwrap();

        assert!(
            primer.is_empty(),
            "native path must not return primer events"
        );
        assert!(
            transcript_agent_prompt.is_none(),
            "native path must not surface a transcript-borne agent prompt"
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
        // 2. Server replies with the transcript carrying the agent prompt.
        push_server_msg(
            &s_tx,
            &ServerMessage::Transcript {
                events: canned.clone(),
                agent_prompt: Some("you are helpful".to_string()),
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
        let (primer, transcript_agent_prompt) = primer_handle.await.unwrap();
        assert_eq!(primer, canned, "primer must be the transcript replay");
        assert_eq!(
            transcript_agent_prompt.as_deref(),
            Some("you are helpful"),
            "transcript-borne agent prompt must be surfaced to the caller",
        );
    }

    #[tokio::test]
    async fn emit_resumed_transcript_path_without_prompt_returns_none() {
        let (mut worker, mut s_rx, s_tx) = paired();
        let prior = SessionId::new();

        let primer_handle = {
            let prior = prior.clone();
            tokio::spawn(async move {
                emit_resumed_and_collect_primer(&mut worker, false, Some(prior), true)
                    .await
                    .unwrap()
            })
        };

        match next_worker_msg(&mut s_rx).await {
            WorkerMessage::RequestTranscript { .. } => {}
            other => panic!("expected RequestTranscript, got {other:?}"),
        }
        // Server replies omitting agent_prompt (None on the wire).
        push_server_msg(
            &s_tx,
            &ServerMessage::Transcript {
                events: Vec::new(),
                agent_prompt: None,
            },
        )
        .await;
        // Drain the Resumed{Transcript} emit.
        let _ = next_worker_msg(&mut s_rx).await;
        let (primer, transcript_agent_prompt) = primer_handle.await.unwrap();
        assert!(primer.is_empty());
        assert!(transcript_agent_prompt.is_none());
    }

    #[tokio::test]
    async fn emit_resumed_fresh_session_emits_nothing() {
        let (mut worker, mut s_rx, _s_tx) = paired();

        // No predecessor session: helper must not send anything.
        let (primer, transcript_agent_prompt) =
            emit_resumed_and_collect_primer(&mut worker, false, None, false)
                .await
                .unwrap();
        assert!(primer.is_empty());
        assert!(transcript_agent_prompt.is_none());
        // Close the sender side so `next` returns None instead of hanging.
        drop(_s_tx);
        drop(worker);
        assert!(s_rx.next().await.is_none(), "no frames must be sent");
    }

    mod unified_cleanup {
        use super::*;
        use crate::worker::report::{RunReport, SessionStateFormat, SessionStateRef, TokenUsage};
        use crate::worker::ws_test_util::{collect_worker_msgs, duplex, push_server_msg};
        use std::io::Write;

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
            let report = RunReport {
                last_message: String::new(),
                usage: TokenUsage::default(),
                model_session_id: Some("sess".to_string()),
                session_state: None,
            };
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
            push_server_msg(&mut server_tx, &ServerMessage::EndSession).await;
            assert!(drain_end_session_headless(&mut ws).await);
        }

        #[tokio::test]
        async fn drain_end_session_headless_returns_false_when_absent() {
            let (mut ws, _server_tx, _server_rx) = duplex();
            // Nothing buffered, timeout returns false.
            assert!(!drain_end_session_headless(&mut ws).await);
        }
    }

    mod headless_forwarder {
        use super::*;
        use crate::worker::ws_test_util::{collect_worker_msgs, duplex};

        #[tokio::test]
        async fn forwards_assistant_and_tool_use_to_ws_in_order() {
            // Feeds three `WorkerEvent`s into the forwarder; expects the two
            // that translate to a `SessionEvent` (AssistantText, ToolUse) to
            // appear on the WS in arrival order, with `Usage` dropped.
            let (ws, _server_tx, mut server_rx) = duplex();
            let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(8);
            let forwarder = spawn_headless_event_forwarder(ws, event_rx);

            event_tx
                .send(WorkerEvent::AssistantText {
                    text: "hello".to_string(),
                })
                .await
                .unwrap();
            event_tx
                .send(WorkerEvent::Usage {
                    usage: TokenUsage::default(),
                })
                .await
                .unwrap();
            event_tx
                .send(WorkerEvent::ToolUse {
                    tool_name: "Bash".to_string(),
                    payload: serde_json::json!({"command": "ls"}),
                })
                .await
                .unwrap();
            drop(event_tx);

            let ws_back = forwarder
                .await
                .expect("forwarder task panicked")
                .expect("forwarder must hand WS back on natural close");
            drop(ws_back);

            let frames = collect_worker_msgs(&mut server_rx).await;
            assert_eq!(frames.len(), 2, "got frames {frames:?}");
            match &frames[0] {
                WorkerMessage::Event {
                    event: SessionEvent::AssistantMessage { content, .. },
                } => assert_eq!(content, "hello"),
                other => panic!("expected AssistantMessage, got {other:?}"),
            }
            match &frames[1] {
                WorkerMessage::Event {
                    event:
                        SessionEvent::ToolUse {
                            tool_name, payload, ..
                        },
                } => {
                    assert_eq!(tool_name, "Bash");
                    assert_eq!(payload, &serde_json::json!({"command": "ls"}));
                }
                other => panic!("expected ToolUse, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn returns_open_ws_on_channel_close_so_cleanup_can_proceed() {
            // Closing the channel (per-wrapper `run` finished, dropped its
            // sender) must hand the WS back to `drive_headless` so the
            // unified cleanup runs on the same socket.
            let (ws, _server_tx, mut server_rx) = duplex();
            let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(4);
            let forwarder = spawn_headless_event_forwarder(ws, event_rx);
            drop(event_tx);

            let mut ws_back = forwarder
                .await
                .expect("forwarder task panicked")
                .expect("forwarder must hand WS back on natural close");

            // Sanity-check the WS is still usable for downstream cleanup.
            ws_back
                .send(WorkerMessage::Event {
                    event: SessionEvent::Closed {
                        timestamp: chrono::Utc::now(),
                    },
                })
                .await
                .expect("WS should still be open");
            drop(ws_back);

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
        async fn await_with_timeout_returns_none_when_orphan_holds_sender() {
            // Simulates the corner case from the wrapper's stdout-pump
            // timeout path: a detached task still holds a `Sender<WorkerEvent>`
            // clone after the per-wrapper `run` returned, so the forwarder's
            // `event_rx.recv().await` never observes the channel closing.
            // `await_headless_forwarder` must bound the wait, abort the
            // forwarder, and return `None` so `drive_headless` skips the
            // cleanup write path (the server-side disconnect fallback caps
            // the log).
            let (ws, _server_tx, _server_rx) = duplex();
            let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(4);
            let forwarder = spawn_headless_event_forwarder(ws, event_rx);

            // Leak the sender into a detached task that never drops it —
            // the orphan stdout pump in production.
            let leak = tokio::spawn(async move {
                let _hold = event_tx;
                std::future::pending::<()>().await;
            });

            let ws_back = await_headless_forwarder(forwarder, Duration::from_millis(50)).await;
            assert!(
                ws_back.is_none(),
                "forwarder timeout must abandon the WS, not hand it back",
            );

            leak.abort();
        }

        #[tokio::test]
        async fn await_with_timeout_returns_ws_on_natural_close() {
            // Sanity check: when the wrapper drops its sender normally, the
            // forwarder's natural-close branch runs and the helper returns
            // the WS — the timeout is just an outer bound, not the common
            // path.
            let (ws, _server_tx, _server_rx) = duplex();
            let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(4);
            let forwarder = spawn_headless_event_forwarder(ws, event_rx);
            drop(event_tx);

            let ws_back = await_headless_forwarder(forwarder, Duration::from_secs(5)).await;
            assert!(
                ws_back.is_some(),
                "natural close must propagate the WS through the helper",
            );
        }

        #[tokio::test]
        async fn drops_ws_when_send_fails_midstream() {
            // Dropping the server-side receiver makes WS sends fail; the
            // forwarder must abandon the WS (return None) so the caller
            // skips cleanup rather than hanging on a broken socket.
            let (ws, server_tx, server_rx) = duplex();
            // Close the server side so the very first send errors out.
            drop(server_tx);
            drop(server_rx);

            let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>(4);
            let forwarder = spawn_headless_event_forwarder(ws, event_rx);
            event_tx
                .send(WorkerEvent::AssistantText {
                    text: "x".to_string(),
                })
                .await
                .unwrap();
            drop(event_tx);

            let ws_back = forwarder.await.expect("forwarder task panicked");
            assert!(ws_back.is_none(), "broken WS must be abandoned");
        }
    }
}
