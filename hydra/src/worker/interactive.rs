use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{Sink, SinkExt, Stream, StreamExt};
use hydra_common::{
    api::v1::conversations::{
        ConversationEvent, ServerMessage, SessionStatePayload, WorkerCatchUp, WorkerConnect,
        WorkerMessage,
    },
    constants::{ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN},
    SessionId,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::Command,
};
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};

use crate::claude_formatter::StreamFormatter;
use crate::client::RelayWebSocket;

/// Run an interactive Claude session, bridging between a relay WebSocket and
/// Claude's stdin/stdout via `--input-format stream-json --output-format stream-json`.
///
/// Resume strategy is two-tier:
///
/// - **Primary** (full-fidelity): when the server returns `session_state` in
///   the catch-up and it parses as a `SessionStatePayload::V1 { transcript:
///   Some(..) }`, the worker writes that transcript blob to the on-disk
///   location Claude expects (`<home>/.claude/projects/<encoded(cwd)>/<UUID>.jsonl`)
///   and spawns Claude with `--resume <session_id>`. Claude reads the
///   restored transcript and the new turn sees the prior tool calls /
///   thinking, not just a summary.
/// - **Fallback** (lossy primer): if no `session_state` is present, the parse
///   fails, the transcript is missing, or the file write fails, the worker
///   falls back to building a context-primer message from the event log and
///   feeding it on stdin. Claude sees a summary of prior turns but no tool
///   calls / thinking. This is the path i-yrgzxm established and is also
///   correct (just lossier).
///
/// In both paths, any *pending* `UserMessage` events from the catch-up — ones
/// that arrived after the prior worker's last `AssistantMessage` — are
/// forwarded to Claude's stdin so it can respond to them.
///
/// # Agent prompt prepend
///
/// `prompt` is the conversation's agent prompt (empty when no agent is bound).
/// Unlike non-interactive mode — where the prompt is passed as a Claude CLI
/// argument and becomes the entire first turn — interactive mode must
/// concatenate the prompt into the **same** Claude turn as the user's first
/// message rather than sending it as a separate turn. The rule is:
///
/// - If `prompt` is non-empty AND the catch-up event log contains no
///   `AssistantMessage` (i.e. Claude has not yet replied in this conversation),
///   the worker prepends `"{prompt}\n\n"` to the first `UserMessage` it pipes
///   to Claude's stdin — either a pending message from the catch-up log or the
///   first message received from the relay if the catch-up was empty. The
///   prepend happens exactly once per worker process.
/// - If `prompt` is empty, behavior is identical to the no-agent case.
/// - If the catch-up already contains an `AssistantMessage`, the prior session
///   has already exposed Claude to the agent prompt; the primer carries the
///   historical context forward and the prepend is suppressed.
#[allow(clippy::too_many_arguments)]
pub async fn run_interactive(
    ws_stream: RelayWebSocket,
    session_id: &SessionId,
    prompt: &str,
    model: Option<&str>,
    env: &HashMap<String, String>,
    working_dir: &Path,
    idle_timeout: Duration,
    conversation_resume_from: Option<usize>,
) -> Result<()> {
    // Validate auth credentials exist.
    let has_anthropic_key = env
        .get(ENV_ANTHROPIC_API_KEY)
        .is_some_and(|v| !v.trim().is_empty());
    let has_oauth_token = env
        .get(ENV_CLAUDE_CODE_OAUTH_TOKEN)
        .is_some_and(|v| !v.trim().is_empty());
    if !has_anthropic_key && !has_oauth_token {
        return Err(anyhow!(
            "Either {ENV_CLAUDE_CODE_OAUTH_TOKEN} or {ENV_ANTHROPIC_API_KEY} must be provided"
        ));
    }

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    println!("WebSocket connected, sending handshake");

    // Send WorkerConnect handshake. The server ignores `resume_from_event_index`
    // for Fresh handshakes and always replies with the full event log; we keep
    // the value for the wire-protocol contract and so the server can log it.
    let handshake = WorkerConnect::Fresh {
        resume_from_event_index: conversation_resume_from,
    };
    let handshake_json =
        serde_json::to_string(&handshake).context("failed to serialize WorkerConnect")?;
    ws_sender
        .send(tungstenite::Message::Text(handshake_json))
        .await
        .context("failed to send WorkerConnect handshake")?;

    // Receive WorkerCatchUp response.
    let catch_up_msg = ws_receiver
        .next()
        .await
        .ok_or_else(|| anyhow!("WebSocket closed before catch-up"))?
        .context("WebSocket error during catch-up")?;

    let catch_up_text = match catch_up_msg {
        tungstenite::Message::Text(text) => text,
        other => return Err(anyhow!("expected text catch-up message, got {other:?}")),
    };

    let server_msg: ServerMessage =
        serde_json::from_str(&catch_up_text).context("failed to parse catch-up message")?;
    let catch_up = match server_msg {
        ServerMessage::CatchUp(cu) => cu,
        other => return Err(anyhow!("expected CatchUp message, got {other:?}")),
    };

    let session_state_bytes = catch_up.session_state.as_ref().map(|b| b.len());
    info!(
        %session_id,
        events = catch_up.events.len(),
        session_state_bytes = ?session_state_bytes,
        "catch_up_received"
    );

    let home_dir = worker_home_dir()?;

    // Try the primary resume path: if the catch-up carries a parsable
    // SessionStatePayload with a transcript blob, restore it to disk so the
    // new Claude process can `--resume <session_id>` against it.
    let primary_resume = try_primary_resume(&catch_up, &home_dir, working_dir);
    let resume_session_id: Option<String> = primary_resume.as_ref().map(|p| p.session_id.clone());

    match (&primary_resume, &catch_up.session_state) {
        (Some(p), _) => info!(
            %session_id,
            resume_session_id = %p.session_id,
            transcript_bytes = p.transcript_bytes,
            "resume_path=primary_transcript"
        ),
        (None, Some(bytes)) => warn!(
            %session_id,
            session_state_bytes = bytes.len(),
            "resume_path=primer_fallback session_state_present_but_unusable"
        ),
        (None, None) => info!(
            %session_id,
            "resume_path=primer_fallback no_session_state"
        ),
    }

    // Spawn Claude in long-lived interactive mode.
    let claude_args = build_claude_args(model, resume_session_id.as_deref());

    eprintln!("Claude CLI args (interactive): {claude_args:?}");

    let mut command = Command::new("claude");
    command
        .args(&claude_args)
        .current_dir(working_dir)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .context("failed to spawn claude in interactive mode")?;

    let pid = child.id().unwrap_or(0);
    println!("Claude interactive process spawned (PID: {pid})");

    let mut claude_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to capture stdin for claude"))?;
    let claude_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture stdout for claude"))?;
    let claude_stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture stderr for claude"))?;

    // Spawn stderr reader (log to stderr).
    tokio::spawn(async move {
        let mut reader = BufReader::new(claude_stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => eprint!("{line}"),
                Err(_) => break,
            }
        }
    });

    // Whether the agent prompt still needs to be prepended to the first
    // UserMessage sent to Claude. We initialize this here — before feeding the
    // catch-up — so the rule is shared between `feed_catch_up` and the relay
    // loop and consumed by whichever sends the first UserMessage. See the
    // doc-comment on this function for the exact prepend rule.
    let mut prompt_prepend = PromptPrepend::new(prompt, &catch_up.events);

    // Feed the catch-up events to Claude's stdin. If we restored a transcript
    // file (primary path), the prior history is already in Claude's view and
    // we only need to forward trailing pending UserMessages. Otherwise we
    // build a primer wrapping the prior transcript and feed it before any
    // pending input.
    feed_catch_up(
        &mut claude_stdin,
        &catch_up.events,
        primary_resume.is_some(),
        &mut prompt_prepend,
    )
    .await?;

    // Set up stdout reader with StreamFormatter.
    let mut stdout_reader = BufReader::new(claude_stdout);

    // Pre-seed claude_session_id with the resumed session id, if any: with
    // `--resume <UUID>` Claude uses the same id, but we want the upload
    // path to work even if no further stdout lines have arrived before
    // suspension.
    let mut claude_session_id: Option<String> = resume_session_id.clone();
    let _ = &session_id; // used for logging context

    // Relay loop: bidirectional message forwarding.
    let exit = relay_loop(
        &mut ws_sender,
        &mut ws_receiver,
        &mut claude_stdin,
        &mut stdout_reader,
        idle_timeout,
        &mut claude_session_id,
        &home_dir,
        working_dir,
        &mut prompt_prepend,
    )
    .await?;

    // Clean shutdown: terminate Claude process.
    match exit {
        LoopExit::IdleSuspended => println!("Shutting down interactive session (idle timeout)"),
        LoopExit::Terminated => println!("Shutting down interactive session (SIGTERM received)"),
        LoopExit::Exited => println!("Shutting down interactive session"),
    }

    #[cfg(unix)]
    if let Some(pgid) = child.id() {
        // SIGTERM the process group.
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGTERM);
        }
    }

    // Give Claude a chance to exit gracefully.
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => {
            println!("Claude process exited with status: {status}");
        }
        Ok(Err(err)) => {
            eprintln!("Error waiting for Claude process: {err}");
        }
        Err(_) => {
            eprintln!("Claude process did not exit within 5s, force killing");
            #[cfg(unix)]
            if let Some(pgid) = child.id() {
                unsafe {
                    libc::kill(-(pgid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
        }
    }

    // Close WebSocket.
    let _ = ws_sender.send(tungstenite::Message::Close(None)).await;

    Ok(())
}

/// Result of `relay_loop`. The caller uses it to log the reason for shutdown;
/// the upload of `SessionStateUpload` (when applicable) is performed inside
/// the loop itself so it goes out on the still-open WebSocket.
#[derive(Debug, PartialEq, Eq)]
enum LoopExit {
    /// Idle timeout fired — the loop sent a Suspending event and (best-effort)
    /// a SessionStateUpload before returning.
    IdleSuspended,
    /// SIGTERM was received (e.g. `/close` killing the worker) — same suspend
    /// sequence as idle.
    Terminated,
    /// Normal end-of-loop (e.g. Claude stdout EOF, WS close). No suspend
    /// event was emitted.
    Exited,
}

/// Outcome of attempting the primary transcript-based resume path.
struct PrimaryResume {
    session_id: String,
    transcript_bytes: usize,
}

/// Try to apply the primary resume path: parse `catch_up.session_state` as
/// `SessionStatePayload::V1 { transcript: Some(..) }`, then write the
/// transcript bytes to the on-disk location Claude reads on `--resume`.
/// Returns `Some(PrimaryResume)` if everything succeeded; `None` otherwise
/// (the caller then falls back to the primer path). A failure on this path
/// is **never** fatal: the conversation still resumes via the fallback, just
/// without the full transcript fidelity.
fn try_primary_resume(
    catch_up: &WorkerCatchUp,
    home_dir: &Path,
    working_dir: &Path,
) -> Option<PrimaryResume> {
    let Some(bytes) = catch_up.session_state.as_deref() else {
        info!("try_primary_resume session_state=absent");
        return None;
    };
    info!(
        bytes = bytes.len(),
        "try_primary_resume session_state=present"
    );

    let payload: SessionStatePayload = match serde_json::from_slice(bytes) {
        Ok(p) => p,
        Err(err) => {
            error!(
                error = %err,
                "try_primary_resume parse_failed falling_back_to_primer"
            );
            return None;
        }
    };

    let (session_id, transcript) = match payload {
        SessionStatePayload::V1 {
            session_id,
            transcript,
        } => (session_id, transcript),
    };

    let bytes = match transcript {
        Some(t) => {
            info!(
                resume_session_id = %session_id,
                transcript_bytes = t.len(),
                "try_primary_resume parsed transcript=present"
            );
            t
        }
        None => {
            warn!(
                resume_session_id = %session_id,
                "try_primary_resume transcript=absent falling_back_to_primer"
            );
            return None;
        }
    };

    let path = transcript_path(home_dir, working_dir, &session_id);
    if let Err(err) = write_transcript_atomic(&path, &bytes) {
        error!(
            transcript_path = %path.display(),
            error = %err,
            "try_primary_resume write_failed falling_back_to_primer"
        );
        return None;
    }
    info!(
        transcript_path = %path.display(),
        transcript_bytes = bytes.len(),
        "try_primary_resume write_succeeded"
    );

    Some(PrimaryResume {
        session_id,
        transcript_bytes: bytes.len(),
    })
}

/// Compute Claude's per-project encoded-cwd directory name. Claude maps each
/// `/` AND each `.` in the working directory's absolute path to a `-` to
/// produce a filesystem-safe directory name under `~/.claude/projects/`. For
/// example `/tmp/.tmpOH7bq5/repo` → `-tmp--tmpOH7bq5-repo` (the leading `/`
/// becomes `-`, and the `/.` between `tmp` and `tmpOH7bq5` becomes `--`).
///
/// Verified empirically against the running worker image — see
/// `encoded_cwd_replaces_slashes_and_dots_with_dashes` for the case used as
/// the source of truth, and the live `~/.claude/projects/` directory in the
/// worker container. If the encoding ever diverges, that test will fail
/// rather than producing a silently-wrong path in production.
fn encoded_cwd(working_dir: &Path) -> String {
    working_dir
        .to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Compute the absolute path Claude reads for `claude --resume <session_id>`.
fn transcript_path(home_dir: &Path, working_dir: &Path, session_id: &str) -> PathBuf {
    home_dir
        .join(".claude")
        .join("projects")
        .join(encoded_cwd(working_dir))
        .join(format!("{session_id}.jsonl"))
}

/// Write `bytes` to `path` atomically: create parent directories, write to a
/// sibling `*.jsonl.tmp` file, then `rename(2)` it over the target. A crash
/// between write and rename leaves the partial file at the tmp path, never as
/// a half-written transcript Claude could mistake for valid input.
fn write_transcript_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "transcript path has no file name component",
            ))
        }
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Resolve the worker's HOME directory. We use `HOME` from the environment
/// rather than `dirs::home_dir()` so the value is the *worker process's* home
/// (where Claude reads `.claude/projects/...`), not whatever `dirs` infers.
fn worker_home_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home))
}

/// Build a `SessionStatePayload` from the captured Claude session id and the
/// current contents of its transcript file. If the file cannot be read, the
/// payload carries `transcript: None`; the resumer will then fall back to the
/// primer path instead of attempting to restore an empty file. The upload is
/// best-effort and never returns an error: a missing file is not a worker
/// failure.
fn build_session_state_payload(
    home_dir: &Path,
    working_dir: &Path,
    session_id: &str,
) -> SessionStatePayload {
    let path = transcript_path(home_dir, working_dir, session_id);
    let transcript = match std::fs::read(&path) {
        Ok(bytes) => {
            info!(
                claude_session_id = session_id,
                transcript_path = %path.display(),
                bytes = bytes.len(),
                "build_session_state_payload read_ok"
            );
            Some(bytes)
        }
        Err(err) => {
            warn!(
                claude_session_id = session_id,
                transcript_path = %path.display(),
                error = %err,
                "build_session_state_payload read_failed uploading_session_id_only"
            );
            None
        }
    };
    SessionStatePayload::V1 {
        session_id: session_id.to_string(),
        transcript,
    }
}

/// Send a `SessionStateUpload` over the WebSocket carrying the serialized
/// payload. Returns `Ok(())` if the send succeeded, otherwise an error (the
/// caller logs but does not propagate — failing to upload state is non-fatal).
async fn send_session_state_upload<Si>(
    ws_sender: &mut Si,
    payload: &SessionStatePayload,
) -> Result<()>
where
    Si: Sink<tungstenite::Message> + Unpin,
{
    let data =
        serde_json::to_vec(payload).context("failed to serialize SessionStatePayload to bytes")?;
    let bytes = data.len();
    let msg = WorkerMessage::SessionStateUpload { data };
    let json = serde_json::to_string(&msg).context("failed to serialize SessionStateUpload")?;
    match ws_sender.send(tungstenite::Message::Text(json)).await {
        Ok(()) => {
            info!(bytes, "send_session_state_upload ws_send_ok");
            Ok(())
        }
        Err(_) => {
            error!(bytes, "send_session_state_upload ws_send_failed");
            Err(anyhow!("WebSocket send of SessionStateUpload failed"))
        }
    }
}

/// Tracks the agent-prompt prepend across `feed_catch_up` and the relay loop.
///
/// At most one `UserMessage` per worker process is prepended — see the
/// doc-comment on [`run_interactive`] for the rule. Built once at the top of
/// `run_interactive`, then threaded through every place that pipes a
/// `UserMessage` to Claude's stdin so the decision lives in exactly one spot.
#[derive(Debug)]
struct PromptPrepend {
    prompt: String,
    pending: bool,
}

impl PromptPrepend {
    fn new(prompt: &str, catch_up_events: &[ConversationEvent]) -> Self {
        let has_assistant = catch_up_events
            .iter()
            .any(|e| matches!(e, ConversationEvent::AssistantMessage { .. }));
        let pending = !prompt.is_empty() && !has_assistant;
        Self {
            prompt: prompt.to_string(),
            pending,
        }
    }

    /// If a prepend is still pending, return `{prompt}\n\n{content}` and mark
    /// it consumed; otherwise return `content` unchanged.
    fn apply(&mut self, content: &str) -> String {
        if self.pending {
            self.pending = false;
            format!("{}\n\n{content}", self.prompt)
        } else {
            content.to_string()
        }
    }
}

/// Feed catch-up events to Claude's stdin.
///
/// - When `using_resumed_transcript` is true, the prior history was restored
///   on disk via `claude --resume`; we must NOT re-feed a primer. Only the
///   trailing pending `UserMessage`s are forwarded.
/// - When false (primer/fallback path), the prior transcript is wrapped into
///   a single primer user message that is fed first, then any pending
///   `UserMessage`s follow.
///
/// `prompt_prepend` is consumed when the first pending `UserMessage` is
/// written. The primer (when emitted) is NOT subject to the prepend: it only
/// applies to actual user-typed messages, and it is suppressed in any case
/// where the primer is sent (which requires prior assistant history).
async fn feed_catch_up<W: AsyncWrite + Unpin>(
    claude_stdin: &mut W,
    events: &[ConversationEvent],
    using_resumed_transcript: bool,
    prompt_prepend: &mut PromptPrepend,
) -> Result<()> {
    let (past_context, pending_user_messages) = partition_events(events);

    if !using_resumed_transcript && !past_context.is_empty() {
        let primer = build_context_primer(&past_context);
        let primer_line = build_claude_input(&primer);
        claude_stdin
            .write_all(primer_line.as_bytes())
            .await
            .context("failed to write context primer to claude stdin")?;
        claude_stdin
            .flush()
            .await
            .context("failed to flush context primer to claude stdin")?;
        println!(
            "Fed context primer ({} prior events) to Claude stdin",
            past_context.len()
        );
    }

    for content in pending_user_messages {
        let to_send = prompt_prepend.apply(content);
        let input_line = build_claude_input(&to_send);
        claude_stdin
            .write_all(input_line.as_bytes())
            .await
            .context("failed to write catch-up user message to claude stdin")?;
        claude_stdin
            .flush()
            .await
            .context("failed to flush catch-up user message to claude stdin")?;
        println!("Fed catch-up user message to Claude stdin");
    }

    Ok(())
}

/// Partition a catch-up event log into:
///
/// 1. `past_context`: every `UserMessage` and `AssistantMessage` up to and
///    including the most recent `AssistantMessage`. This is the part Claude
///    needs to see as historical context. System events (Suspending, Resumed,
///    Closed) are filtered out.
/// 2. `pending`: `UserMessage` content strings that appeared **after** the
///    last `AssistantMessage`. These are messages the prior worker did not
///    get to reply to and that the resumed Claude should respond to.
///
/// If there are no `AssistantMessage` events at all, every `UserMessage` is
/// treated as pending (no prior context yet) and `past_context` is empty.
fn partition_events(events: &[ConversationEvent]) -> (Vec<&ConversationEvent>, Vec<&str>) {
    let last_assistant_idx = events
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e, ConversationEvent::AssistantMessage { .. }))
        .map(|(i, _)| i);

    let mut past_context: Vec<&ConversationEvent> = Vec::new();
    let mut pending: Vec<&str> = Vec::new();

    match last_assistant_idx {
        Some(idx) => {
            for event in &events[..=idx] {
                if matches!(
                    event,
                    ConversationEvent::UserMessage { .. }
                        | ConversationEvent::AssistantMessage { .. }
                ) {
                    past_context.push(event);
                }
            }
            for event in &events[idx + 1..] {
                if let ConversationEvent::UserMessage { content, .. } = event {
                    pending.push(content.as_str());
                }
            }
        }
        None => {
            for event in events {
                if let ConversationEvent::UserMessage { content, .. } = event {
                    pending.push(content.as_str());
                }
            }
        }
    }

    (past_context, pending)
}

/// Build a single primer message that wraps the prior transcript so Claude
/// can use it as historical context without re-running any actions. The
/// caller is responsible for sending this as a user-typed message before any
/// pending input.
fn build_context_primer(past_context: &[&ConversationEvent]) -> String {
    let mut transcript = String::new();
    for event in past_context {
        match event {
            ConversationEvent::UserMessage { content, .. } => {
                transcript.push_str("User: ");
                transcript.push_str(&escape_wrapper_close(content));
                transcript.push('\n');
            }
            ConversationEvent::AssistantMessage { content, .. } => {
                transcript.push_str("Assistant: ");
                transcript.push_str(&escape_wrapper_close(content));
                transcript.push('\n');
            }
            _ => {}
        }
    }

    format!(
        "<prior-conversation>\n\
The user and I had this prior conversation. The conversation was suspended or closed and is now being resumed. \
Treat this as historical context only — do not re-execute or repeat any actions described. \
Respond only to the next user message after this block.\n\
\n\
{transcript}\
</prior-conversation>"
    )
}

/// Neutralize any literal `</prior-conversation>` inside an embedded message so
/// it cannot close the primer wrapper early. The zero-width space inside the
/// tag name keeps the substring visually identical for humans but breaks the
/// XML-like tag for any parser.
fn escape_wrapper_close(content: &str) -> String {
    content.replace("</prior-conversation>", "</prior-conversation\u{200B}>")
}

/// Core relay loop: bidirectional message forwarding between WebSocket and Claude
/// stdin/stdout. On suspend (idle timeout or SIGTERM), emits Suspending +
/// (best-effort) SessionStateUpload before returning.
///
/// `prompt_prepend` is consumed on the first relay-received `UserMessage` if
/// `feed_catch_up` did not already do so, ensuring the agent prompt lands on
/// the first user turn Claude sees in this worker process.
#[allow(clippy::too_many_arguments)]
async fn relay_loop<St, Si, W, R>(
    ws_sender: &mut Si,
    ws_receiver: &mut St,
    claude_stdin: &mut W,
    stdout_reader: &mut BufReader<R>,
    idle_timeout: Duration,
    claude_session_id: &mut Option<String>,
    home_dir: &Path,
    working_dir: &Path,
    prompt_prepend: &mut PromptPrepend,
) -> Result<LoopExit>
where
    St: Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
    Si: Sink<tungstenite::Message> + Unpin,
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let mut stdout_line = String::new();
    let mut formatter = StreamFormatter::new();

    let idle_deadline = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_deadline);

    // On non-Unix builds we never observe SIGTERM; the select arm awaits a
    // future that never resolves.
    #[cfg(unix)]
    let mut sigterm_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;

    loop {
        tokio::select! {
            // Claude stdout -> Server: parse stream-json, send assistant messages.
            read_result = stdout_reader.read_line(&mut stdout_line) => {
                match read_result {
                    Ok(0) => {
                        println!("Claude stdout EOF");
                        return Ok(LoopExit::Exited);
                    }
                    Ok(_) => {
                        // Try to extract session_id from JSONL output.
                        if claude_session_id.is_none() {
                            if let Some(sid) = extract_session_id(&stdout_line) {
                                println!("Extracted Claude session_id: {sid}");
                                *claude_session_id = Some(sid);
                            }
                        }

                        // Process with StreamFormatter for logging.
                        let formatted_lines = formatter.handle_line(&stdout_line);
                        for line in &formatted_lines {
                            print!("{line}");
                        }

                        // Extract assistant text and send to server.
                        if let Some(text) = extract_assistant_text(&stdout_line) {
                            if !text.is_empty() {
                                let event = ConversationEvent::AssistantMessage {
                                    content: text,
                                    timestamp: Utc::now(),
                                };
                                let msg = WorkerMessage::Event { event };
                                let json = serde_json::to_string(&msg)
                                    .context("failed to serialize worker message")?;
                                if ws_sender
                                    .send(tungstenite::Message::Text(json))
                                    .await
                                    .is_err()
                                {
                                    println!("WebSocket closed while sending assistant message");
                                    return Ok(LoopExit::Exited);
                                }
                            }
                        }

                        stdout_line.clear();
                    }
                    Err(err) => {
                        eprintln!("Error reading Claude stdout: {err}");
                        return Ok(LoopExit::Exited);
                    }
                }
            }

            // Server -> Claude: receive user messages from WebSocket.
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(ServerMessage::Event { event }) => {
                                if let ConversationEvent::UserMessage { content, .. } = event {
                                    // Reset idle timer on user input.
                                    idle_deadline.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                                    let to_send = prompt_prepend.apply(&content);
                                    let input_line = build_claude_input(&to_send);
                                    if claude_stdin
                                        .write_all(input_line.as_bytes())
                                        .await
                                        .is_err()
                                    {
                                        eprintln!("Failed to write to Claude stdin (process may have exited)");
                                        return Ok(LoopExit::Exited);
                                    }
                                    if claude_stdin.flush().await.is_err() {
                                        eprintln!("Failed to flush Claude stdin");
                                        return Ok(LoopExit::Exited);
                                    }
                                    println!("Forwarded user message to Claude stdin");
                                }
                            }
                            Ok(ServerMessage::CatchUp(_)) => {
                                eprintln!("Unexpected CatchUp message during relay loop");
                            }
                            Err(err) => {
                                eprintln!("Failed to parse server message: {err}");
                            }
                        }
                    }
                    Some(Ok(tungstenite::Message::Ping(data))) => {
                        let _ = ws_sender
                            .send(tungstenite::Message::Pong(data))
                            .await;
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        println!("WebSocket closed by server");
                        return Ok(LoopExit::Exited);
                    }
                    Some(Ok(_)) => {
                        // Ignore binary, pong, etc.
                    }
                    Some(Err(err)) => {
                        eprintln!("WebSocket error: {err}");
                        return Ok(LoopExit::Exited);
                    }
                }
            }

            // Idle timeout: suspend the session when no user input is received.
            _ = &mut idle_deadline => {
                println!("Idle timeout reached ({idle_timeout:?}), suspending session");
                emit_suspend(
                    ws_sender,
                    "idle_timeout",
                    claude_session_id.as_deref(),
                    home_dir,
                    working_dir,
                )
                .await;
                return Ok(LoopExit::IdleSuspended);
            }

            // SIGTERM (e.g. `/close` kill_job): emit the same suspend
            // sequence as idle so the prior worker's state is uploaded
            // before the container exits.
            _ = await_sigterm(
                #[cfg(unix)]
                &mut sigterm_signal,
            ) => {
                println!("SIGTERM received, suspending session");
                emit_suspend(
                    ws_sender,
                    "sigterm",
                    claude_session_id.as_deref(),
                    home_dir,
                    working_dir,
                )
                .await;
                return Ok(LoopExit::Terminated);
            }
        }
    }
}

/// Suspend-emission helper: writes a `Suspending` event followed by a
/// best-effort `SessionStateUpload` carrying the transcript file. Each step
/// is independent — if the upload fails (file missing, WS dropped) we still
/// return so the loop can exit. Suspending must be sent first because the
/// server uses it to transition the conversation to Idle.
async fn emit_suspend<Si>(
    ws_sender: &mut Si,
    reason: &str,
    claude_session_id: Option<&str>,
    home_dir: &Path,
    working_dir: &Path,
) where
    Si: Sink<tungstenite::Message> + Unpin,
{
    info!(
        reason,
        has_claude_session_id = claude_session_id.is_some(),
        "emit_suspend entry"
    );
    let suspending_event = ConversationEvent::Suspending {
        reason: reason.to_string(),
        timestamp: Utc::now(),
    };
    let suspending_msg = WorkerMessage::Event {
        event: suspending_event,
    };
    if let Ok(json) = serde_json::to_string(&suspending_msg) {
        if ws_sender
            .send(tungstenite::Message::Text(json))
            .await
            .is_err()
        {
            error!(reason, "emit_suspend ws_send_suspending_failed ws_closed");
            return;
        }
    }

    let Some(session_id) = claude_session_id else {
        warn!(
            reason,
            "emit_suspend transcript_upload_skipped — Claude never emitted a session_id"
        );
        return;
    };

    let payload = build_session_state_payload(home_dir, working_dir, session_id);
    match send_session_state_upload(ws_sender, &payload).await {
        Ok(()) => info!(
            reason,
            claude_session_id = session_id,
            "emit_suspend upload_ok"
        ),
        Err(err) => error!(
            reason,
            claude_session_id = session_id,
            error = %err,
            "emit_suspend upload_failed"
        ),
    }
}

/// Await the next SIGTERM, or (on non-Unix) never resolve. We deliberately
/// take the signal by `&mut` so the handle is installed exactly once for the
/// lifetime of the loop and is shared across iterations.
#[cfg(unix)]
async fn await_sigterm(sig: &mut tokio::signal::unix::Signal) {
    sig.recv().await;
}

#[cfg(not(unix))]
async fn await_sigterm() {
    futures::future::pending::<()>().await;
}

/// Build the Claude CLI argument list. When `resume_session_id` is `Some`,
/// the worker has just restored a transcript file to Claude's project state
/// directory and wants Claude to continue that session.
fn build_claude_args(model: Option<&str>, resume_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--verbose".to_string(),
    ];
    if let Some(model) = model {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    if let Some(session_id) = resume_session_id {
        args.push("--resume".to_string());
        args.push(session_id.to_string());
    }
    args
}

/// Build a stream-json input line for Claude's stdin.
fn build_claude_input(content: &str) -> String {
    let input = serde_json::json!({
        "type": "user",
        "session_id": "",
        "parent_tool_use_id": null,
        "message": {
            "role": "user",
            "content": content
        }
    });
    format!("{input}\n")
}

/// Extract the `session_id` field from a Claude JSONL output line.
fn extract_session_id(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    value
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract assistant text content from a Claude stream-json output line.
/// Returns Some(text) if the line is an assistant message with text content.
fn extract_assistant_text(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let content = value.get("message")?.get("content")?.as_array()?;

    let mut text_parts = Vec::new();
    for chunk in content {
        if chunk.get("type")?.as_str()? == "text" {
            if let Some(text) = chunk.get("text").and_then(|v| v.as_str()) {
                text_parts.push(text.to_string());
            }
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;
    use tempfile::TempDir;

    /// Install a process-wide tracing subscriber that routes events through
    /// `print!` so `cargo test -- --nocapture` surfaces them. Tests that
    /// exercise the new `info!`/`warn!`/`error!` sites added in
    /// `interactive.rs` for the resume-path instrumentation call this so the
    /// log lines are visible during local debugging and CI failures.
    fn init_test_tracing() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = tracing_subscriber::fmt()
                .with_test_writer()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .try_init();
        });
    }

    fn user_msg(content: &str) -> ConversationEvent {
        ConversationEvent::UserMessage {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn assistant_msg(content: &str) -> ConversationEvent {
        ConversationEvent::AssistantMessage {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn suspending() -> ConversationEvent {
        ConversationEvent::Suspending {
            reason: "idle_timeout".to_string(),
            timestamp: Utc::now(),
        }
    }

    fn closed() -> ConversationEvent {
        ConversationEvent::Closed {
            timestamp: Utc::now(),
        }
    }

    fn resumed() -> ConversationEvent {
        ConversationEvent::Resumed {
            session_id: SessionId::new(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn build_claude_input_formats_correctly() {
        let input = build_claude_input("Hello, Claude!");
        let parsed: serde_json::Value = serde_json::from_str(&input).unwrap();
        assert_eq!(parsed["type"], "user");
        assert_eq!(parsed["session_id"], "");
        assert!(parsed["parent_tool_use_id"].is_null());
        assert_eq!(parsed["message"]["role"], "user");
        assert_eq!(parsed["message"]["content"], "Hello, Claude!");
    }

    #[test]
    fn extract_session_id_from_output() {
        let line = r#"{"type":"assistant","session_id":"abc-123","message":{"content":[]}}"#;
        assert_eq!(extract_session_id(line), Some("abc-123".to_string()));
    }

    #[test]
    fn extract_session_id_returns_none_for_empty() {
        let line = r#"{"type":"assistant","session_id":"","message":{"content":[]}}"#;
        assert_eq!(extract_session_id(line), None);
    }

    #[test]
    fn extract_session_id_returns_none_when_missing() {
        let line = r#"{"type":"assistant","message":{"content":[]}}"#;
        assert_eq!(extract_session_id(line), None);
    }

    #[test]
    fn extract_assistant_text_from_text_block() {
        let line =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello!"}]}}"#;
        assert_eq!(extract_assistant_text(line), Some("Hello!".to_string()));
    }

    #[test]
    fn extract_assistant_text_skips_tool_use() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#;
        assert_eq!(extract_assistant_text(line), None);
    }

    #[test]
    fn extract_assistant_text_multiple_blocks() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Part 1"},{"type":"text","text":"Part 2"}]}}"#;
        assert_eq!(
            extract_assistant_text(line),
            Some("Part 1\nPart 2".to_string())
        );
    }

    #[test]
    fn build_claude_args_omits_resume_when_session_id_is_none() {
        let args = build_claude_args(None, None);
        assert!(
            !args.iter().any(|a| a == "--resume"),
            "no resume_session_id means no --resume flag"
        );
    }

    #[test]
    fn build_claude_args_emits_resume_when_session_id_is_some() {
        let args = build_claude_args(None, Some("abc-123"));
        let resume_idx = args
            .iter()
            .position(|a| a == "--resume")
            .expect("--resume should be present");
        assert_eq!(args[resume_idx + 1], "abc-123");
    }

    #[test]
    fn build_claude_args_includes_model_when_set() {
        let args = build_claude_args(Some("opus"), None);
        assert!(args.iter().any(|a| a == "--model"));
        assert!(args.iter().any(|a| a == "opus"));
    }

    #[test]
    fn worker_connect_serializes_resume_from_event_index() {
        let handshake = WorkerConnect::Fresh {
            resume_from_event_index: Some(5),
        };
        let json = serde_json::to_string(&handshake).unwrap();
        assert!(json.contains("\"resume_from_event_index\":5"));
    }

    #[test]
    fn extract_assistant_text_ignores_user_messages() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;
        assert_eq!(extract_assistant_text(line), None);
    }

    #[test]
    fn partition_events_returns_past_context_and_pending_user_messages() {
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
            suspending(),
        ];
        let (past, pending) = partition_events(&events);
        assert_eq!(
            past.len(),
            4,
            "past should contain msg1..reply2, got {past:?}"
        );
        assert!(matches!(
            past[0],
            ConversationEvent::UserMessage { content, .. } if content == "msg1"
        ));
        assert!(matches!(
            past[1],
            ConversationEvent::AssistantMessage { content, .. } if content == "reply1"
        ));
        assert!(matches!(
            past[2],
            ConversationEvent::UserMessage { content, .. } if content == "msg2"
        ));
        assert!(matches!(
            past[3],
            ConversationEvent::AssistantMessage { content, .. } if content == "reply2"
        ));
        assert_eq!(pending, vec!["msg3"]);
    }

    #[test]
    fn partition_events_handles_empty_history() {
        let (past, pending) = partition_events(&[]);
        assert!(past.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn partition_events_handles_no_assistant_yet() {
        let events = vec![user_msg("msg1")];
        let (past, pending) = partition_events(&events);
        assert!(
            past.is_empty(),
            "no AssistantMessage means no past context; got {past:?}"
        );
        assert_eq!(pending, vec!["msg1"]);
    }

    #[test]
    fn partition_events_skips_system_events_in_past_context() {
        // A Suspending or Closed event appearing in the middle of the
        // transcript (rare but possible across multiple resumes) must not be
        // included in past_context — only User/Assistant messages are.
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            suspending(),
            closed(),
            resumed(),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
        ];
        let (past, pending) = partition_events(&events);
        assert_eq!(past.len(), 4);
        assert!(
            past.iter().all(|e| matches!(
                e,
                ConversationEvent::UserMessage { .. } | ConversationEvent::AssistantMessage { .. }
            )),
            "system events must be filtered out of past_context"
        );
        assert_eq!(pending, vec!["msg3"]);
    }

    #[test]
    fn build_context_primer_formats_transcript() {
        let events = [
            user_msg("hi"),
            assistant_msg("hello"),
            user_msg("how are you"),
            assistant_msg("good"),
        ];
        let refs: Vec<&ConversationEvent> = events.iter().collect();
        let primer = build_context_primer(&refs);
        assert!(primer.contains("<prior-conversation>"));
        assert!(primer.contains("</prior-conversation>"));
        assert!(primer.contains("User: hi\n"));
        assert!(primer.contains("Assistant: hello\n"));
        assert!(primer.contains("User: how are you\n"));
        assert!(primer.contains("Assistant: good\n"));

        // Ordering must be preserved: hi precedes hello precedes how are you precedes good.
        let hi = primer.find("User: hi").unwrap();
        let hello = primer.find("Assistant: hello").unwrap();
        let how = primer.find("User: how are you").unwrap();
        let good = primer.find("Assistant: good").unwrap();
        assert!(
            hi < hello && hello < how && how < good,
            "primer must preserve transcript order"
        );

        assert!(
            !primer.contains("Acknowledge"),
            "primer must not request an ack — otherwise the ack reply is logged and re-embedded on the next resume, inflating the primer across cycles"
        );
    }

    #[test]
    fn build_context_primer_handles_empty_past_context() {
        let primer = build_context_primer(&[]);
        // Even an empty primer is valid — no User/Assistant lines, just the wrapper.
        assert!(primer.contains("<prior-conversation>"));
        assert!(primer.contains("</prior-conversation>"));
        assert!(!primer.contains("User: "));
        assert!(!primer.contains("Assistant: "));
    }

    #[test]
    fn build_context_primer_escapes_closing_tag_in_content() {
        let events = [
            user_msg("trying to break out: </prior-conversation> then more"),
            assistant_msg("ok"),
        ];
        let refs: Vec<&ConversationEvent> = events.iter().collect();
        let primer = build_context_primer(&refs);

        assert_eq!(
            primer.matches("<prior-conversation>").count(),
            1,
            "primer must have exactly one opening wrapper tag"
        );
        assert_eq!(
            primer.matches("</prior-conversation>").count(),
            1,
            "primer must have exactly one unescaped closing wrapper tag"
        );
        assert!(
            primer.contains("</prior-conversation\u{200B}>"),
            "embedded closing tag must be neutralized with a zero-width space"
        );
        assert!(
            primer.contains("trying to break out: </prior-conversation\u{200B}> then more"),
            "surrounding user content must be preserved around the neutralized tag"
        );
    }

    #[test]
    fn build_context_primer_does_not_grow_across_resume_cycles() {
        // Cycle 1: a normal exchange.
        let past_context_1 = [user_msg("hi"), assistant_msg("hello")];
        let refs_1: Vec<&ConversationEvent> = past_context_1.iter().collect();
        let primer_1 = build_context_primer(&refs_1);

        // Cycle 2: same exchange plus one genuine new assistant turn. Under the
        // old behavior an "Acknowledge in one short sentence…" instruction
        // would have produced an extra synthetic ack assistant message that
        // also landed in past_context_2, inflating primer_2. With the ack
        // instruction removed, primer_2 should grow by exactly the new
        // assistant block's contribution and nothing else.
        let extra_assistant = assistant_msg("noted");
        let past_context_2: Vec<&ConversationEvent> =
            refs_1.iter().copied().chain([&extra_assistant]).collect();
        let primer_2 = build_context_primer(&past_context_2);

        let expected_new_block = "Assistant: noted\n";
        assert_eq!(
            primer_2.len(),
            primer_1.len() + expected_new_block.len(),
            "primer must grow by exactly the new assistant block, with no extra preamble inflation"
        );
        assert!(primer_2.ends_with("</prior-conversation>"));
    }

    #[test]
    fn encoded_cwd_replaces_slashes_and_dots_with_dashes() {
        // Canonical case from the issue description and verified against the
        // worker image's actual `~/.claude/projects/` directory layout.
        assert_eq!(
            encoded_cwd(Path::new("/tmp/.tmpOH7bq5/repo")),
            "-tmp--tmpOH7bq5-repo"
        );
        // No dots: just slashes get rewritten.
        assert_eq!(encoded_cwd(Path::new("/home/worker")), "-home-worker");
        // Multiple dots in a leaf: each becomes a dash.
        assert_eq!(encoded_cwd(Path::new("/var/lib/x.y.z")), "-var-lib-x-y-z");
        // Trailing slash variant should still be deterministic.
        assert_eq!(encoded_cwd(Path::new("/a/b/c/")), "-a-b-c-");
    }

    #[test]
    fn transcript_path_layout_matches_claude_convention() {
        let p = transcript_path(
            Path::new("/home/worker"),
            Path::new("/tmp/.tmpOH7bq5/repo"),
            "abc-123",
        );
        assert_eq!(
            p,
            PathBuf::from("/home/worker/.claude/projects/-tmp--tmpOH7bq5-repo/abc-123.jsonl")
        );
    }

    #[test]
    fn write_transcript_atomic_creates_parent_dirs_and_writes_bytes() {
        let tmp = TempDir::new().unwrap();
        let path = tmp
            .path()
            .join(".claude")
            .join("projects")
            .join("-tmp-repo")
            .join("sid.jsonl");
        write_transcript_atomic(&path, b"line1\nline2\n").unwrap();

        let contents = std::fs::read(&path).unwrap();
        assert_eq!(contents, b"line1\nline2\n");
        // The tmp file used during the atomic rename must not linger.
        let tmp_sibling = path.with_file_name("sid.jsonl.tmp");
        assert!(
            !tmp_sibling.exists(),
            "atomic write should rename the tmp file, not leave it at {tmp_sibling:?}"
        );
    }

    #[test]
    fn write_transcript_atomic_overwrites_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("dir").join("sid.jsonl");
        write_transcript_atomic(&path, b"first").unwrap();
        write_transcript_atomic(&path, b"second").unwrap();
        let contents = std::fs::read(&path).unwrap();
        assert_eq!(contents, b"second");
    }

    #[test]
    fn try_primary_resume_returns_none_when_session_state_missing() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let cu = WorkerCatchUp {
            events: vec![],
            session_state: None,
        };
        assert!(try_primary_resume(&cu, tmp.path(), Path::new("/work")).is_none());
    }

    #[test]
    fn try_primary_resume_returns_none_when_payload_unparseable() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let cu = WorkerCatchUp {
            events: vec![],
            session_state: Some(b"not-json".to_vec()),
        };
        assert!(try_primary_resume(&cu, tmp.path(), Path::new("/work")).is_none());
    }

    #[test]
    fn try_primary_resume_returns_none_when_transcript_absent() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let payload = SessionStatePayload::V1 {
            session_id: "abc".to_string(),
            transcript: None,
        };
        let cu = WorkerCatchUp {
            events: vec![],
            session_state: Some(serde_json::to_vec(&payload).unwrap()),
        };
        assert!(try_primary_resume(&cu, tmp.path(), Path::new("/work")).is_none());
    }

    #[test]
    fn try_primary_resume_writes_transcript_and_returns_session_id() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let working_dir = Path::new("/tmp/.tmpOH7bq5/repo");
        let session_id = "abc-123";
        let bytes = b"{\"type\":\"summary\"}\n".to_vec();

        let payload = SessionStatePayload::V1 {
            session_id: session_id.to_string(),
            transcript: Some(bytes.clone()),
        };
        let cu = WorkerCatchUp {
            events: vec![],
            session_state: Some(serde_json::to_vec(&payload).unwrap()),
        };
        let result = try_primary_resume(&cu, tmp.path(), working_dir)
            .expect("primary resume should succeed");
        assert_eq!(result.session_id, session_id);
        assert_eq!(result.transcript_bytes, bytes.len());

        let expected_path = transcript_path(tmp.path(), working_dir, session_id);
        let on_disk = std::fs::read(&expected_path).unwrap();
        assert_eq!(on_disk, bytes);
    }

    #[test]
    fn build_session_state_payload_with_transcript_on_disk() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let working_dir = Path::new("/tmp/.tmpOH7bq5/repo");
        let session_id = "sid-1";
        let bytes = b"hello\n".to_vec();
        let path = transcript_path(tmp.path(), working_dir, session_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();

        let payload = build_session_state_payload(tmp.path(), working_dir, session_id);
        match payload {
            SessionStatePayload::V1 {
                session_id: sid,
                transcript,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(transcript, Some(bytes));
            }
        }
    }

    #[test]
    fn build_session_state_payload_without_transcript_on_disk() {
        init_test_tracing();
        let tmp = TempDir::new().unwrap();
        let payload =
            build_session_state_payload(tmp.path(), Path::new("/no/such/cwd"), "unknown-sid");
        match payload {
            SessionStatePayload::V1 {
                session_id,
                transcript,
            } => {
                assert_eq!(session_id, "unknown-sid");
                assert!(transcript.is_none(), "missing file → transcript=None");
            }
        }
    }

    // Helper to collect all messages sent to the ws sink.
    fn collect_ws_messages(
        rx: &mut futures::channel::mpsc::UnboundedReceiver<tungstenite::Message>,
    ) -> Vec<tungstenite::Message> {
        let mut messages = Vec::new();
        while let Ok(Some(msg)) = rx.try_next() {
            messages.push(msg);
        }
        messages
    }

    fn parse_worker_message(msg: &tungstenite::Message) -> WorkerMessage {
        match msg {
            tungstenite::Message::Text(t) => serde_json::from_str(t).unwrap(),
            other => panic!("expected text message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_idle_timeout_fires_when_no_input() {
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Keep _stdout_write alive so reads block (pending), not EOF.
        let (stdout_read, _stdout_write) = tokio::io::duplex(1024);
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut claude_stdin = tokio::io::sink();
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            Path::new("/work"),
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::IdleSuspended);

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        // With no captured session_id, only Suspending is sent.
        assert_eq!(messages.len(), 1, "expected exactly one Suspending message");
        match parse_worker_message(&messages[0]) {
            WorkerMessage::Event {
                event: ConversationEvent::Suspending { reason, .. },
            } => {
                assert_eq!(reason, "idle_timeout");
            }
            other => panic!("expected Suspending event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_idle_timeout_resets_on_user_message() {
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;

        // Send one UserMessage, then go pending (no more messages).
        let user_msg = ServerMessage::Event {
            event: ConversationEvent::UserMessage {
                content: "test input".to_string(),
                timestamp: Utc::now(),
            },
        };
        let user_msg_json = serde_json::to_string(&user_msg).unwrap();
        let mut ws_receiver = futures::stream::iter(vec![Ok::<_, tungstenite::Error>(
            tungstenite::Message::Text(user_msg_json),
        )])
        .chain(futures::stream::pending());

        let (stdout_read, _stdout_write) = tokio::io::duplex(1024);
        let mut stdout_reader = BufReader::new(stdout_read);

        // Use duplex for stdin so we can verify the message was forwarded.
        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            Path::new("/work"),
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::IdleSuspended);

        // Verify the user message was forwarded to Claude stdin.
        drop(claude_stdin);
        let mut buf = String::new();
        let mut reader = BufReader::new(&mut stdin_read);
        reader.read_line(&mut buf).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(parsed["type"], "user");
        assert_eq!(parsed["message"]["content"], "test input");

        // Verify Suspending event was sent.
        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert_eq!(messages.len(), 1);
        match parse_worker_message(&messages[0]) {
            WorkerMessage::Event {
                event: ConversationEvent::Suspending { reason, .. },
            } => {
                assert_eq!(reason, "idle_timeout");
            }
            other => panic!("expected Suspending event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_idle_timeout_uploads_session_state_with_transcript() {
        // When a session_id was captured AND a transcript file exists on
        // disk, idle suspension must emit Suspending followed by a
        // SessionStateUpload carrying the file bytes inside a V1 payload.
        init_test_tracing();
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let working_dir = Path::new("/tmp/.tmpOH7bq5/repo");
        let session_id_str = "test-session-123";
        let transcript_bytes = b"{\"type\":\"summary\",\"x\":1}\n".to_vec();
        let path = transcript_path(tmp.path(), working_dir, session_id_str);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &transcript_bytes).unwrap();

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Inject a stdout line so the session_id is extracted in the loop.
        let (stdout_read, mut stdout_write) = tokio::io::duplex(4096);
        let session_line = format!(
            r#"{{"type":"assistant","session_id":"{session_id_str}","message":{{"content":[]}}}}"#
        );
        stdout_write
            .write_all(format!("{session_line}\n").as_bytes())
            .await
            .unwrap();

        let mut claude_stdin = tokio::io::sink();
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            working_dir,
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::IdleSuspended);
        assert_eq!(session_id.as_deref(), Some(session_id_str));

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert_eq!(
            messages.len(),
            2,
            "expected Suspending + SessionStateUpload, got {messages:?}"
        );
        match parse_worker_message(&messages[0]) {
            WorkerMessage::Event {
                event: ConversationEvent::Suspending { reason, .. },
            } => {
                assert_eq!(reason, "idle_timeout");
            }
            other => panic!("expected Suspending event, got {other:?}"),
        }
        match parse_worker_message(&messages[1]) {
            WorkerMessage::SessionStateUpload { data } => {
                let payload: SessionStatePayload = serde_json::from_slice(&data).unwrap();
                match payload {
                    SessionStatePayload::V1 {
                        session_id,
                        transcript,
                    } => {
                        assert_eq!(session_id, session_id_str);
                        assert_eq!(transcript, Some(transcript_bytes));
                    }
                }
            }
            other => panic!("expected SessionStateUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_idle_timeout_uploads_payload_with_transcript_none_when_file_missing() {
        // A session_id was captured but the transcript file does not exist
        // (e.g. Claude crashed before any line was flushed). We should still
        // upload a payload with `transcript: None` so the server has the
        // session_id even though the resumer must fall back.
        init_test_tracing();
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let working_dir = Path::new("/tmp/.tmpOH7bq5/repo");
        let session_id_str = "no-file-session";

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        let (stdout_read, mut stdout_write) = tokio::io::duplex(4096);
        let session_line = format!(
            r#"{{"type":"assistant","session_id":"{session_id_str}","message":{{"content":[]}}}}"#
        );
        stdout_write
            .write_all(format!("{session_line}\n").as_bytes())
            .await
            .unwrap();

        let mut claude_stdin = tokio::io::sink();
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            working_dir,
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::IdleSuspended);
        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert_eq!(
            messages.len(),
            2,
            "expected Suspending + SessionStateUpload"
        );
        match parse_worker_message(&messages[1]) {
            WorkerMessage::SessionStateUpload { data } => {
                let payload: SessionStatePayload = serde_json::from_slice(&data).unwrap();
                match payload {
                    SessionStatePayload::V1 {
                        session_id,
                        transcript,
                    } => {
                        assert_eq!(session_id, session_id_str);
                        assert!(transcript.is_none(), "no file on disk → transcript None");
                    }
                }
            }
            other => panic!("expected SessionStateUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_claude_stdout_eof_exits_without_suspending() {
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Close stdout immediately — EOF.
        let (stdout_read, stdout_write) = tokio::io::duplex(1024);
        drop(stdout_write);
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut claude_stdin = tokio::io::sink();
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            Path::new("/work"),
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::Exited);

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert!(messages.is_empty(), "no Suspending event should be sent");
    }

    #[tokio::test]
    async fn test_feed_catch_up_primer_path_sends_primer_then_pending() {
        // Fallback path (no transcript on disk): the first stdin line must be
        // the primer wrapping prior transcript and the second must be msg3.
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
        ];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &events);
        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut first_line = String::new();
        reader.read_line(&mut first_line).await.unwrap();
        let first: serde_json::Value = serde_json::from_str(&first_line).unwrap();
        assert_eq!(first["type"], "user");
        let primer_content = first["message"]["content"].as_str().unwrap();
        assert!(primer_content.contains("<prior-conversation>"));
        assert!(primer_content.contains("User: msg1"));
        assert!(primer_content.contains("Assistant: reply1"));
        assert!(primer_content.contains("User: msg2"));
        assert!(primer_content.contains("Assistant: reply2"));
        assert!(!primer_content.contains("User: msg3"));

        let mut second_line = String::new();
        reader.read_line(&mut second_line).await.unwrap();
        let second: serde_json::Value = serde_json::from_str(&second_line).unwrap();
        assert_eq!(second["type"], "user");
        assert_eq!(second["message"]["content"], "msg3");

        let mut trailing = String::new();
        let n = reader.read_line(&mut trailing).await.unwrap();
        assert_eq!(n, 0, "no trailing input expected, got {trailing:?}");
    }

    #[tokio::test]
    async fn test_feed_catch_up_resumed_path_skips_primer_and_sends_pending_only() {
        // Primary path: Claude already has the transcript on disk via
        // --resume; do NOT emit a primer. Still forward any trailing
        // pending UserMessages (here, msg3).
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
        ];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &events);
        feed_catch_up(&mut claude_stdin, &events, true, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut first_line = String::new();
        reader.read_line(&mut first_line).await.unwrap();
        let first: serde_json::Value = serde_json::from_str(&first_line).unwrap();
        assert_eq!(first["type"], "user");
        let content = first["message"]["content"].as_str().unwrap();
        assert_eq!(
            content, "msg3",
            "first line on the resumed path is the pending user message, not a primer"
        );
        assert!(
            !content.contains("<prior-conversation>"),
            "primer must NOT be sent on the resumed-transcript path"
        );

        let mut trailing = String::new();
        let n = reader.read_line(&mut trailing).await.unwrap();
        assert_eq!(n, 0, "exactly one input line expected");
    }

    #[tokio::test]
    async fn test_feed_catch_up_resumed_path_with_no_pending_writes_nothing() {
        // No pending messages and we're on the resumed-transcript path — the
        // function should not write anything at all.
        let events = vec![user_msg("msg1"), assistant_msg("reply1")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &events);
        feed_catch_up(&mut claude_stdin, &events, true, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.unwrap();
        assert_eq!(n, 0, "no input expected, got {line:?}");
    }

    #[tokio::test]
    async fn test_feed_catch_up_no_primer_when_no_assistant_history() {
        let events = vec![user_msg("hello")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &events);
        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["message"]["content"], "hello");
        assert!(
            !parsed["message"]["content"]
                .as_str()
                .unwrap()
                .contains("<prior-conversation>"),
            "no primer expected without prior assistant history"
        );

        let mut trailing = String::new();
        let n = reader.read_line(&mut trailing).await.unwrap();
        assert_eq!(n, 0, "exactly one input line expected");
    }

    #[tokio::test]
    async fn test_feed_catch_up_empty_events_does_nothing() {
        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &[]);
        feed_catch_up(&mut claude_stdin, &[], false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.unwrap();
        assert_eq!(n, 0, "no input expected for empty catch-up");
    }

    const AGENT_PROMPT: &str = "You are the SWE agent.";

    #[tokio::test]
    async fn prompt_prepended_to_first_pending_user_message_in_catch_up() {
        // Catch-up contains a single UserMessage and no AssistantMessage —
        // the agent prompt must be concatenated into the same Claude turn
        // as that UserMessage, with `\n\n` separating the two.
        let events = vec![user_msg("write a fizzbuzz")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new(AGENT_PROMPT, &events);
        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        let content = parsed["message"]["content"].as_str().unwrap();
        assert_eq!(content, format!("{AGENT_PROMPT}\n\nwrite a fizzbuzz"));

        let mut trailing = String::new();
        let n = reader.read_line(&mut trailing).await.unwrap();
        assert_eq!(n, 0, "exactly one input line expected");
        assert!(
            !prompt_prepend.pending,
            "prepend must be marked consumed after the first send"
        );
    }

    #[tokio::test]
    async fn prompt_prepended_to_first_relay_user_message_when_catch_up_empty() {
        // Catch-up has zero events: the prepend must land on the first
        // UserMessage received from the relay, and not on the second.
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let (ws_tx, _ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;

        let msg1 = ServerMessage::Event {
            event: ConversationEvent::UserMessage {
                content: "first".to_string(),
                timestamp: Utc::now(),
            },
        };
        let msg2 = ServerMessage::Event {
            event: ConversationEvent::UserMessage {
                content: "second".to_string(),
                timestamp: Utc::now(),
            },
        };
        let stream = vec![
            Ok::<_, tungstenite::Error>(tungstenite::Message::Text(
                serde_json::to_string(&msg1).unwrap(),
            )),
            Ok::<_, tungstenite::Error>(tungstenite::Message::Text(
                serde_json::to_string(&msg2).unwrap(),
            )),
        ];
        let mut ws_receiver = futures::stream::iter(stream).chain(futures::stream::pending());

        let (stdout_read, _stdout_write) = tokio::io::duplex(1024);
        let mut stdout_reader = BufReader::new(stdout_read);

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new(AGENT_PROMPT, &[]);
        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            Path::new("/work"),
            &mut prompt_prepend,
        )
        .await
        .unwrap();

        assert_eq!(result, LoopExit::IdleSuspended);

        drop(claude_stdin);
        let mut reader = BufReader::new(&mut stdin_read);

        let mut line1 = String::new();
        reader.read_line(&mut line1).await.unwrap();
        let v1: serde_json::Value = serde_json::from_str(&line1).unwrap();
        assert_eq!(
            v1["message"]["content"].as_str().unwrap(),
            format!("{AGENT_PROMPT}\n\nfirst")
        );

        let mut line2 = String::new();
        reader.read_line(&mut line2).await.unwrap();
        let v2: serde_json::Value = serde_json::from_str(&line2).unwrap();
        assert_eq!(
            v2["message"]["content"].as_str().unwrap(),
            "second",
            "second relay message must NOT receive the prepend"
        );
    }

    #[tokio::test]
    async fn prompt_not_prepended_when_catch_up_contains_assistant_message() {
        // The catch-up already has an AssistantMessage, so the prior session
        // exposed Claude to the agent prompt. The primer carries history
        // forward; the trailing pending UserMessage must NOT be prepended.
        let events = vec![user_msg("msg1"), assistant_msg("reply1"), user_msg("msg2")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new(AGENT_PROMPT, &events);
        assert!(
            !prompt_prepend.pending,
            "constructor must suppress prepend when catch-up has an AssistantMessage"
        );

        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);

        // First line: primer wrapping msg1/reply1.
        let mut primer_line = String::new();
        reader.read_line(&mut primer_line).await.unwrap();
        let primer: serde_json::Value = serde_json::from_str(&primer_line).unwrap();
        let primer_content = primer["message"]["content"].as_str().unwrap();
        assert!(primer_content.contains("<prior-conversation>"));
        assert!(
            !primer_content.starts_with(AGENT_PROMPT),
            "agent prompt must NOT prefix the primer"
        );

        // Second line: trailing pending user message verbatim.
        let mut pending_line = String::new();
        reader.read_line(&mut pending_line).await.unwrap();
        let pending: serde_json::Value = serde_json::from_str(&pending_line).unwrap();
        assert_eq!(pending["message"]["content"].as_str().unwrap(), "msg2");
    }

    #[tokio::test]
    async fn prompt_not_prepended_when_prompt_is_empty() {
        // Empty prompt → behavior identical to today: no extra newlines, no
        // whitespace-only payload sent.
        let events = vec![user_msg("hello")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;

        let mut prompt_prepend = PromptPrepend::new("", &events);
        assert!(
            !prompt_prepend.pending,
            "empty prompt must suppress prepend even with no assistant history"
        );

        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["message"]["content"].as_str().unwrap(), "hello");
    }

    #[tokio::test]
    async fn prompt_prepended_only_once_across_pending_and_relay() {
        // Catch-up has a pending UserMessage AND another UserMessage arrives
        // via the relay. Only the first (the pending one) gets the prepend.
        tokio::time::pause();

        let tmp = TempDir::new().unwrap();
        let (ws_tx, _ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;

        let events = vec![user_msg("pending-1")];

        let relay_msg = ServerMessage::Event {
            event: ConversationEvent::UserMessage {
                content: "relay-1".to_string(),
                timestamp: Utc::now(),
            },
        };
        let mut ws_receiver = futures::stream::iter(vec![Ok::<_, tungstenite::Error>(
            tungstenite::Message::Text(serde_json::to_string(&relay_msg).unwrap()),
        )])
        .chain(futures::stream::pending());

        let (stdout_read, _stdout_write) = tokio::io::duplex(1024);
        let mut stdout_reader = BufReader::new(stdout_read);

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;
        let mut session_id = None;

        let mut prompt_prepend = PromptPrepend::new(AGENT_PROMPT, &events);

        feed_catch_up(&mut claude_stdin, &events, false, &mut prompt_prepend)
            .await
            .unwrap();
        assert!(
            !prompt_prepend.pending,
            "feed_catch_up should have consumed the prepend"
        );

        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
            tmp.path(),
            Path::new("/work"),
            &mut prompt_prepend,
        )
        .await
        .unwrap();
        assert_eq!(result, LoopExit::IdleSuspended);

        drop(claude_stdin);
        let mut reader = BufReader::new(&mut stdin_read);

        let mut line1 = String::new();
        reader.read_line(&mut line1).await.unwrap();
        let v1: serde_json::Value = serde_json::from_str(&line1).unwrap();
        assert_eq!(
            v1["message"]["content"].as_str().unwrap(),
            format!("{AGENT_PROMPT}\n\npending-1")
        );

        let mut line2 = String::new();
        reader.read_line(&mut line2).await.unwrap();
        let v2: serde_json::Value = serde_json::from_str(&line2).unwrap();
        assert_eq!(
            v2["message"]["content"].as_str().unwrap(),
            "relay-1",
            "the relay-side message must NOT receive a second prepend"
        );
    }

    #[test]
    fn prompt_prepend_new_initializes_pending_only_when_no_assistant_and_prompt_nonempty() {
        // Truth table for the constructor.
        let user_only = vec![user_msg("hi")];
        let with_assistant = vec![user_msg("hi"), assistant_msg("hello")];

        assert!(PromptPrepend::new("p", &user_only).pending);
        assert!(!PromptPrepend::new("p", &with_assistant).pending);
        assert!(!PromptPrepend::new("", &user_only).pending);
        assert!(!PromptPrepend::new("", &with_assistant).pending);
        assert!(!PromptPrepend::new("", &[]).pending);
        assert!(PromptPrepend::new("p", &[]).pending);
    }
}
