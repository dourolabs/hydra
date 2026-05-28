//! `Claude` per-worker wrapper plus the Claude-native I/O vocabulary it
//! consumes/produces. See `designs/worker-model-commands-refactor.md` §2.
//!
//! Per-worker setup (env validation, MCP-config tempfile) happens in
//! [`Claude::new`]; per-call I/O happens in [`Claude::run`] (batch) and
//! [`Claude::run_interactive`] (long-lived).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use hydra_common::{
    api::v1::conversations::SessionStatePayload,
    constants::{ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN},
    SessionId,
};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

use super::claude_formatter::StreamFormatter;
use crate::worker::report::{
    MaterializeError, NativeResume, RunReport, SessionStateFormat, SessionStateRef, TokenUsage,
};

/// Grace period after the main process exits before killing the process group.
const PROCESS_GROUP_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Time to wait for SIGTERM to take effect before sending SIGKILL.
const SIGTERM_WAIT: Duration = Duration::from_secs(5);

/// Timeout for stdout/stderr pipe reads after process group kill.
const PIPE_READ_TIMEOUT: Duration = Duration::from_secs(10);

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

/// One event emitted by Claude on its stdout (stream-json output), parsed into
/// a typed shape.
#[derive(Debug, Clone)]
pub enum ClaudeEvent {
    Assistant {
        text: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    },
    SystemInit {
        session_id: String,
    },
    /// A `tool_use` content block observed in an `assistant` stream-json line.
    ToolUse {
        tool_name: String,
        payload: serde_json::Value,
    },
    Raw {
        value: serde_json::Value,
    },
}

/// Per-worker Claude wrapper. Holds all state that does not need to change
/// between invocations (auth env, MCP config tempfile, home dir, idle timeout).
pub struct Claude {
    model: Option<String>,
    working_dir: PathBuf,
    home_dir: PathBuf,
    env: HashMap<String, String>,
    mcp_config: Option<McpConfig>,
    _mcp_tempdir: Option<TempDir>,
    idle_timeout: Duration,
}

/// On-disk MCP config written for Claude's `--mcp-config` flag.
pub struct McpConfig {
    pub on_disk_path: PathBuf,
}

impl Claude {
    /// Construct a per-worker Claude wrapper.
    ///
    /// Validates that at least one of `ANTHROPIC_API_KEY` /
    /// `CLAUDE_CODE_OAUTH_TOKEN` is present, writes the MCP config to a
    /// tempfile if provided, and stashes everything for later `run` /
    /// `run_interactive` calls.
    pub async fn new(
        model: Option<String>,
        working_dir: PathBuf,
        home_dir: PathBuf,
        env: HashMap<String, String>,
        mcp_config_json: Option<&str>,
        idle_timeout: Duration,
    ) -> Result<Self> {
        let has_anthropic_key = env
            .get(ENV_ANTHROPIC_API_KEY)
            .is_some_and(|v| !v.trim().is_empty());
        let has_oauth_token = env
            .get(ENV_CLAUDE_CODE_OAUTH_TOKEN)
            .is_some_and(|v| !v.trim().is_empty());
        if !has_anthropic_key && !has_oauth_token {
            return Err(anyhow!(
                "Either {ENV_CLAUDE_CODE_OAUTH_TOKEN} or {ENV_ANTHROPIC_API_KEY} must be provided in the job context environment"
            ));
        }

        let (mcp_tempdir, mcp_config) = match mcp_config_json {
            Some(json) => {
                let tmp_dir = tempfile::Builder::new()
                    .prefix("mcp-config")
                    .tempdir()
                    .context("failed to create temporary directory for MCP config")?;
                let config_path = tmp_dir.path().join("mcp-config.json");
                fs::write(&config_path, json)
                    .await
                    .context("failed to write MCP config to temp file")?;
                (
                    Some(tmp_dir),
                    Some(McpConfig {
                        on_disk_path: config_path,
                    }),
                )
            }
            None => (None, None),
        };

        Ok(Self {
            model,
            working_dir,
            home_dir,
            env,
            mcp_config,
            _mcp_tempdir: mcp_tempdir,
            idle_timeout,
        })
    }

    /// Attempt to materialize a resume blob into a native Claude resume
    /// handle. Per design `sessions-worker-run-interface.md` §3.4–3.5:
    ///
    /// * deserialize the bytes as [`SessionStatePayload::V1`],
    /// * write the embedded transcript to Claude's expected path
    ///   (`~/.claude/projects/<encoded-cwd>/<UUID>.jsonl`),
    /// * return `Ok(NativeResume::Claude(ClaudeResume::SessionId(uuid)))`.
    ///
    /// Returns [`MaterializeError::WrongFormat`] if the bytes do not parse as
    /// this wrapper's payload (e.g. cross-model handoff from a Codex
    /// session), [`MaterializeError::MissingTranscript`] if the payload
    /// parses but carries no transcript (the bare session id alone cannot
    /// resume Claude on a fresh worker — there is no on-disk transcript to
    /// resume against), and [`MaterializeError::IoError`] if writing the
    /// on-disk transcript fails. The dispatcher treats all error variants
    /// identically and falls back to transcript replay.
    pub fn try_materialize(&self, state_bytes: &[u8]) -> Result<NativeResume, MaterializeError> {
        let payload: SessionStatePayload =
            serde_json::from_slice(state_bytes).map_err(|_| MaterializeError::WrongFormat)?;
        let SessionStatePayload::V1 {
            session_id,
            transcript,
        } = payload;
        let bytes = transcript.ok_or(MaterializeError::MissingTranscript)?;
        let target = transcript_path(&self.home_dir, &self.working_dir, &session_id);
        write_transcript_atomic(&target, &bytes)?;
        Ok(NativeResume::Claude(ClaudeResume::SessionId(session_id)))
    }

    /// Run a one-shot, non-interactive Claude invocation and return the
    /// resulting `RunReport`.
    pub async fn run(&mut self, prompt: &str, resume: Option<ClaudeResume>) -> Result<RunReport> {
        let resume_uuid = match resume {
            Some(ClaudeResume::SessionId(uuid)) => Some(uuid),
            Some(ClaudeResume::TranscriptFile(path)) => Some(install_claude_transcript_file(
                &self.home_dir,
                &self.working_dir,
                &path,
            )?),
            None => None,
        };

        let mcp_path = self.mcp_config.as_ref().map(|m| m.on_disk_path.as_path());
        let args = build_claude_args(
            prompt,
            self.model.as_deref(),
            mcp_path,
            resume_uuid.as_deref(),
        );

        let mut command = Command::new("claude");
        command.args(&args);
        command.current_dir(&self.working_dir).envs(&self.env);
        #[cfg(unix)]
        command.process_group(0);

        eprintln!("Claude CLI args: {args:?}");

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude command")?;

        let spawn_time = Instant::now();
        let pid = child.id().unwrap_or(0);
        println!("Claude process spawned (PID: {pid})");

        #[cfg(unix)]
        let child_pgid = child.id();

        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture stdout for claude command"))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("failed to capture stderr for claude command"))?;

        let stderr_handle = tokio::spawn(async move {
            let mut stderr_buf = Vec::new();
            tokio::io::BufReader::new(child_stderr)
                .read_to_end(&mut stderr_buf)
                .await
                .context("failed to read claude stderr")?;
            Ok::<Vec<u8>, anyhow::Error>(stderr_buf)
        });

        let mut stdout_handle = tokio::spawn(async move {
            let mut formatter = StreamFormatter::new();
            let mut reader = BufReader::new(child_stdout);
            let mut stdout_buf = String::new();
            let mut stdout_writer = io::stdout();
            let mut line = String::new();
            let mut session_id: Option<String> = None;
            loop {
                line.clear();
                let read = reader
                    .read_line(&mut line)
                    .await
                    .context("failed to read claude stdout")?;
                if read == 0 {
                    let elapsed = spawn_time.elapsed().as_secs_f64();
                    println!("Claude stdout EOF reached (PID: {pid}, elapsed: {elapsed:.2}s)");
                    break;
                }
                if let Some(sid) = extract_session_id(&line) {
                    session_id = Some(sid);
                }
                for formatted in formatter.handle_line(&line) {
                    stdout_writer
                        .write_all(formatted.as_bytes())
                        .await
                        .context("failed to stream claude stdout")?;
                    stdout_writer
                        .flush()
                        .await
                        .context("failed to flush claude stdout")?;
                    stdout_buf.push_str(&formatted);
                }
            }
            let last_message = formatter.last_assistant_text().map(str::to_owned);
            let usage = formatter.aggregated_usage().clone();
            Ok::<(String, Option<String>, Option<String>, TokenUsage), anyhow::Error>((
                stdout_buf,
                last_message,
                session_id,
                usage,
            ))
        });

        println!("Waiting for claude process to exit (PID: {pid})…");
        let status = child
            .wait()
            .await
            .context("failed waiting for claude command to finish")?;
        let wait_elapsed = spawn_time.elapsed().as_secs_f64();
        println!(
            "Claude process exited (PID: {pid}, status: {status}, elapsed: {wait_elapsed:.2}s)"
        );

        let (stdout_buf, last_message, session_id, usage) =
            match tokio::time::timeout(PROCESS_GROUP_GRACE_PERIOD, &mut stdout_handle).await {
                Ok(join_result) => join_result.context("failed to join claude stdout task")??,
                Err(_) => {
                    #[cfg(unix)]
                    if let Some(pgid) = child_pgid {
                        eprintln!(
                            "claude process exited but stdout pipe still open; \
                             killing process group {pgid}"
                        );
                        kill_process_group(pgid).await;
                    }

                    match tokio::time::timeout(PIPE_READ_TIMEOUT, stdout_handle).await {
                        Ok(join_result) => {
                            join_result.context("failed to join claude stdout task")??
                        }
                        Err(_) => {
                            let timeout = PIPE_READ_TIMEOUT;
                            eprintln!(
                                "stdout pipe read timed out after {timeout:?} — \
                                 dropping handle and proceeding with partial output"
                            );
                            (String::new(), None, None, TokenUsage::default())
                        }
                    }
                }
            };

        let stderr_result = tokio::time::timeout(PIPE_READ_TIMEOUT, stderr_handle).await;
        let stderr_buf = match stderr_result {
            Ok(join_result) => join_result.context("failed to join claude stderr task")??,
            Err(_) => {
                let timeout = PIPE_READ_TIMEOUT;
                eprintln!(
                    "stderr pipe read timed out after {timeout:?} — \
                     dropping handle and proceeding without stderr"
                );
                Vec::new()
            }
        };

        if !status.success() {
            return Err(anyhow!(
                "claude command failed with status {}. stdout: {}. stderr: {}",
                status,
                stdout_buf,
                String::from_utf8_lossy(&stderr_buf)
            ));
        }

        let last_message = last_message.unwrap_or(stdout_buf);

        let session_state = match &session_id {
            Some(sid) => session_state_if_exists(
                transcript_path(&self.home_dir, &self.working_dir, sid),
                SessionStateFormat::ClaudeJsonl,
            ),
            None => None,
        };

        Ok(RunReport {
            last_message,
            usage,
            model_session_id: session_id,
            session_state,
        })
    }

    /// Run an interactive Claude session: spawn `claude` once, pump
    /// `ClaudeUserMessage`s from `input` to its stdin and parse its stdout
    /// into `ClaudeEvent`s emitted on `output`.
    ///
    /// Termination contract:
    /// * Closing `input` (the relay-side translator dropping its sender)
    ///   signals "no more input". We drop Claude's stdin handle, which causes
    ///   Claude to see stdin EOF and exit. After stdin is dropped we keep
    ///   draining stdout so any final assistant output / token usage line is
    ///   surfaced on `output`.
    /// * Dropping `output` (the relay-side translator stopping consumption) is
    ///   a best-effort signal: we keep reading Claude's stdout (so the JSONL
    ///   transcript on disk remains complete) but stop sending events.
    /// * Stdout EOF (Claude exited) is the natural end.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_interactive(
        &mut self,
        mut input: mpsc::Receiver<ClaudeUserMessage>,
        output: mpsc::Sender<ClaudeEvent>,
        session_id: &SessionId,
        prompt: &str,
        resume: Option<ClaudeResume>,
    ) -> Result<RunReport> {
        // `prompt` is not consumed here — the relay-side adapter applies the
        // agent-prompt prepend to the first `WorkerInputMessage` before we
        // ever see it. Kept on the signature for symmetry with `run`.
        let _ = prompt;
        let _ = session_id;

        let resume_uuid = match resume {
            Some(ClaudeResume::SessionId(uuid)) => Some(uuid),
            Some(ClaudeResume::TranscriptFile(path)) => Some(install_claude_transcript_file(
                &self.home_dir,
                &self.working_dir,
                &path,
            )?),
            None => None,
        };

        let claude_args =
            build_interactive_claude_args(self.model.as_deref(), resume_uuid.as_deref());
        eprintln!("Claude CLI args (interactive): {claude_args:?}");

        let mut command = Command::new("claude");
        command
            .args(&claude_args)
            .current_dir(&self.working_dir)
            .envs(&self.env)
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

        let claude_stdin = child
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

        let mut stdout_reader = BufReader::new(claude_stdout);
        let mut stdout_line = String::new();
        let mut formatter = StreamFormatter::new();
        let mut claude_session_id: Option<String> = resume_uuid.clone();
        // Track whether `claude_stdin` is still open. We drop it the moment the
        // input channel closes so Claude observes EOF on stdin.
        let mut stdin_open = true;
        let mut stdin_taken: Option<tokio::process::ChildStdin> = Some(claude_stdin);

        let idle_timeout = self.idle_timeout;
        let idle_deadline = tokio::time::sleep(idle_timeout);
        tokio::pin!(idle_deadline);

        loop {
            tokio::select! {
                read_result = stdout_reader.read_line(&mut stdout_line) => {
                    match read_result {
                        Ok(0) => {
                            println!("Claude interactive stdout EOF (PID: {pid})");
                            break;
                        }
                        Ok(_) => {
                            if claude_session_id.is_none() {
                                if let Some(sid) = extract_session_id(&stdout_line) {
                                    println!("Extracted Claude session_id: {sid}");
                                    claude_session_id = Some(sid.clone());
                                    let _ = output.send(ClaudeEvent::SystemInit { session_id: sid }).await;
                                }
                            }

                            for line in formatter.handle_line(&stdout_line) {
                                print!("{line}");
                            }

                            for event in parse_claude_events(&stdout_line) {
                                if output.send(event).await.is_err() {
                                    // Output channel closed — keep draining
                                    // stdout but stop sending events.
                                }
                            }

                            stdout_line.clear();
                        }
                        Err(err) => {
                            eprintln!("Error reading Claude stdout: {err}");
                            break;
                        }
                    }
                }

                msg = input.recv(), if stdin_open => {
                    match msg {
                        Some(ClaudeUserMessage { content }) => {
                            idle_deadline
                                .as_mut()
                                .reset(tokio::time::Instant::now() + idle_timeout);
                            if let Some(stdin) = stdin_taken.as_mut() {
                                let input_line = build_claude_input(&content);
                                if stdin.write_all(input_line.as_bytes()).await.is_err() {
                                    eprintln!("Failed to write to Claude stdin (process may have exited)");
                                    stdin_open = false;
                                    stdin_taken = None;
                                    continue;
                                }
                                if stdin.flush().await.is_err() {
                                    eprintln!("Failed to flush Claude stdin");
                                    stdin_open = false;
                                    stdin_taken = None;
                                    continue;
                                }
                                println!("Forwarded user message to Claude stdin");
                            }
                        }
                        None => {
                            // Input channel closed; drop stdin so Claude sees
                            // EOF and exits cleanly. Continue draining stdout.
                            println!("Input channel closed; dropping Claude stdin");
                            stdin_open = false;
                            stdin_taken = None;
                        }
                    }
                }

                _ = &mut idle_deadline => {
                    println!("Interactive idle timeout ({idle_timeout:?}); ending session");
                    stdin_open = false;
                    stdin_taken = None;
                    // After closing stdin, fall through; the next iteration
                    // will await stdout EOF.
                    idle_deadline
                        .as_mut()
                        .reset(tokio::time::Instant::now() + Duration::from_secs(3600));
                }
            }
        }

        // Best-effort SIGTERM to ensure the process group is reaped.
        #[cfg(unix)]
        if let Some(pgid) = child.id() {
            unsafe {
                libc::kill(-(pgid as i32), libc::SIGTERM);
            }
        }

        match tokio::time::timeout(SIGTERM_WAIT, child.wait()).await {
            Ok(Ok(status)) => {
                println!("Claude interactive process exited with status: {status}");
            }
            Ok(Err(err)) => {
                eprintln!("Error waiting for Claude interactive process: {err}");
            }
            Err(_) => {
                eprintln!("Claude did not exit within {SIGTERM_WAIT:?}, force killing");
                #[cfg(unix)]
                if let Some(pgid) = child.id() {
                    unsafe {
                        libc::kill(-(pgid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
            }
        }

        let last_message = formatter
            .last_assistant_text()
            .map(str::to_owned)
            .unwrap_or_default();
        let usage = formatter.aggregated_usage().clone();

        let session_state = match &claude_session_id {
            Some(sid) => session_state_if_exists(
                transcript_path(&self.home_dir, &self.working_dir, sid),
                SessionStateFormat::ClaudeJsonl,
            ),
            None => None,
        };

        Ok(RunReport {
            last_message,
            usage,
            model_session_id: claude_session_id,
            session_state,
        })
    }
}

/// Sends SIGTERM then SIGKILL to a process group.
#[cfg(unix)]
async fn kill_process_group(pgid: u32) {
    let neg_pgid = -(pgid as i32);
    unsafe {
        libc::kill(neg_pgid, libc::SIGTERM);
    }
    tokio::time::sleep(SIGTERM_WAIT).await;
    unsafe {
        libc::kill(neg_pgid, libc::SIGKILL);
    }
}

/// Builds the Claude CLI argument list for a one-shot run.
///
/// Uses `--` to separate options from the positional prompt argument.
pub(crate) fn build_claude_args(
    prompt: &str,
    model: Option<&str>,
    mcp_config_path: Option<&Path>,
    resume_session_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    if let Some(model) = model {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    if let Some(mcp_path) = mcp_config_path {
        args.push("--mcp-config".to_string());
        args.push(mcp_path.to_string_lossy().to_string());
    }
    if let Some(session_id) = resume_session_id {
        args.push("--resume".to_string());
        args.push(session_id.to_string());
    }
    args.push("--".to_string());
    args.push(prompt.to_string());
    args
}

/// Builds the Claude CLI argument list for a long-lived interactive session.
pub(crate) fn build_interactive_claude_args(
    model: Option<&str>,
    resume_session_id: Option<&str>,
) -> Vec<String> {
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
pub(crate) fn build_claude_input(content: &str) -> String {
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
pub(crate) fn extract_session_id(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    value
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract assistant text content from a Claude stream-json output line.
pub(crate) fn extract_assistant_text(line: &str) -> Option<String> {
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

/// Parse a single Claude stream-json line into zero or more `ClaudeEvent`s.
///
/// Each line may produce:
/// * `SystemInit` (if `type=system && subtype=init`),
/// * `Assistant` (if `type=assistant` and any text blocks),
/// * `ToolUse` (one per `tool_use` content block in an assistant line),
/// * `Usage` (if the line carries `message.usage`),
/// * `Raw` (catch-all for anything else parseable as JSON).
fn parse_claude_events(line: &str) -> Vec<ClaudeEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut events = Vec::new();

    match value.get("type").and_then(|v| v.as_str()) {
        Some("system") => {
            if value.get("subtype").and_then(|v| v.as_str()) == Some("init") {
                if let Some(sid) = value.get("session_id").and_then(|v| v.as_str()) {
                    if !sid.is_empty() {
                        events.push(ClaudeEvent::SystemInit {
                            session_id: sid.to_string(),
                        });
                        return events;
                    }
                }
            }
            events.push(ClaudeEvent::Raw {
                value: value.clone(),
            });
        }
        Some("assistant") => {
            if let Some(text) = extract_assistant_text(line) {
                if !text.is_empty() {
                    events.push(ClaudeEvent::Assistant { text });
                }
            }
            if let Some(content) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for chunk in content {
                    if chunk.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                        continue;
                    }
                    let tool_name = chunk
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown_tool")
                        .to_string();
                    let payload = chunk
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    events.push(ClaudeEvent::ToolUse { tool_name, payload });
                }
            }
            if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                let read_u64 = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
                events.push(ClaudeEvent::Usage {
                    input_tokens: read_u64("input_tokens"),
                    output_tokens: read_u64("output_tokens"),
                    cache_read_input_tokens: read_u64("cache_read_input_tokens"),
                    cache_creation_input_tokens: read_u64("cache_creation_input_tokens"),
                });
            }
            if events.is_empty() {
                events.push(ClaudeEvent::Raw { value });
            }
        }
        _ => {
            events.push(ClaudeEvent::Raw { value });
        }
    }

    events
}

/// Compute Claude's per-project encoded-cwd directory name.
pub(crate) fn encoded_cwd(working_dir: &Path) -> String {
    working_dir
        .to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Compute the absolute path Claude reads for `claude --resume <session_id>`.
pub(crate) fn transcript_path(home_dir: &Path, working_dir: &Path, session_id: &str) -> PathBuf {
    home_dir
        .join(".claude")
        .join("projects")
        .join(encoded_cwd(working_dir))
        .join(format!("{session_id}.jsonl"))
}

/// Write `bytes` to `path` atomically: create parent directories, write to a
/// sibling `*.jsonl.tmp` file, then `rename(2)` it over the target.
pub(crate) fn write_transcript_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
            ));
        }
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Install a Claude transcript file at the on-disk location Claude expects for
/// `--resume <UUID>`. Reads the source file, extracts the session UUID from
/// the first JSONL line's `session_id` field, writes the bytes atomically to
/// `<home>/.claude/projects/<encoded(cwd)>/<UUID>.jsonl`, and returns the
/// UUID.
pub(crate) fn install_claude_transcript_file(
    home_dir: &Path,
    working_dir: &Path,
    src: &Path,
) -> Result<String> {
    let bytes =
        std::fs::read(src).with_context(|| format!("failed to read transcript file {src:?}"))?;
    let first_line = bytes
        .split(|b| *b == b'\n')
        .next()
        .ok_or_else(|| anyhow!("transcript file {src:?} is empty"))?;
    let first_line_str = std::str::from_utf8(first_line)
        .context("transcript first line is not valid UTF-8")?
        .trim();
    let value: serde_json::Value =
        serde_json::from_str(first_line_str).context("transcript first line is not valid JSON")?;
    let session_id = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("transcript first line missing non-empty session_id"))?
        .to_string();
    let target = transcript_path(home_dir, working_dir, &session_id);
    write_transcript_atomic(&target, &bytes)
        .with_context(|| format!("failed to install transcript at {target:?}"))?;
    Ok(session_id)
}

/// Wrap a candidate session-state path into `Some(SessionStateRef)` iff it
/// exists on disk; return `None` otherwise.
pub(crate) fn session_state_if_exists(
    local_path: PathBuf,
    format: SessionStateFormat,
) -> Option<SessionStateRef> {
    if local_path.exists() {
        Some(SessionStateRef { local_path, format })
    } else {
        tracing::debug!(
            path = %local_path.display(),
            ?format,
            "session-state path does not exist; returning None"
        );
        None
    }
}

/// Resolve the worker's HOME directory.
#[allow(dead_code)]
pub(crate) fn worker_home_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_claude_args_without_mcp_config() {
        let args = build_claude_args("Do something", None, None, None);
        assert_eq!(
            args,
            vec![
                "--print",
                "--dangerously-skip-permissions",
                "--verbose",
                "--output-format",
                "stream-json",
                "--",
                "Do something",
            ]
        );
    }

    #[test]
    fn test_build_claude_args_with_mcp_config() {
        let mcp_path = Path::new("/tmp/mcp-config/mcp-config.json");
        let args = build_claude_args(
            "Do something",
            Some("claude-sonnet-4-6"),
            Some(mcp_path),
            None,
        );
        assert_eq!(
            args,
            vec![
                "--print",
                "--dangerously-skip-permissions",
                "--verbose",
                "--output-format",
                "stream-json",
                "--model",
                "claude-sonnet-4-6",
                "--mcp-config",
                "/tmp/mcp-config/mcp-config.json",
                "--",
                "Do something",
            ]
        );
    }

    #[test]
    fn test_build_claude_args_prompt_after_separator() {
        let mcp_path = Path::new("/tmp/config.json");
        let prompt = "You are a tester agent responsible for running tests...";
        let args = build_claude_args(prompt, None, Some(mcp_path), None);

        let separator_pos = args.iter().position(|a| a == "--").unwrap();
        let prompt_pos = args.iter().position(|a| a == prompt).unwrap();
        assert!(
            prompt_pos == separator_pos + 1,
            "prompt must immediately follow '--' separator"
        );

        let mcp_config_pos = args.iter().position(|a| a == "--mcp-config").unwrap();
        assert!(
            mcp_config_pos < separator_pos,
            "--mcp-config must come before '--' separator"
        );
    }

    #[test]
    fn build_interactive_claude_args_omits_resume_when_session_id_is_none() {
        let args = build_interactive_claude_args(None, None);
        assert!(!args.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn build_interactive_claude_args_emits_resume_when_session_id_is_some() {
        let args = build_interactive_claude_args(None, Some("abc-123"));
        let idx = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[idx + 1], "abc-123");
    }

    #[test]
    fn build_interactive_claude_args_includes_model_when_set() {
        let args = build_interactive_claude_args(Some("opus"), None);
        assert!(args.iter().any(|a| a == "--model"));
        assert!(args.iter().any(|a| a == "opus"));
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
    fn parse_claude_events_system_init_emits_session_init() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#;
        let events = parse_claude_events(line);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ClaudeEvent::SystemInit { session_id } => assert_eq!(session_id, "abc-123"),
            other => panic!("expected SystemInit, got {other:?}"),
        }
    }

    #[test]
    fn parse_claude_events_assistant_emits_text() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        let events = parse_claude_events(line);
        assert!(events
            .iter()
            .any(|e| matches!(e, ClaudeEvent::Assistant { text } if text == "hi")));
    }

    #[test]
    fn parse_claude_events_assistant_with_usage_emits_usage() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":3,"output_tokens":1}}}"#;
        let events = parse_claude_events(line);
        assert!(events.iter().any(|e| matches!(
            e,
            ClaudeEvent::Usage {
                input_tokens: 3,
                output_tokens: 1,
                ..
            }
        )));
    }

    #[test]
    fn parse_claude_events_unknown_lands_in_raw() {
        let line = r#"{"type":"weird","what":"ever"}"#;
        let events = parse_claude_events(line);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ClaudeEvent::Raw { .. }));
    }

    #[test]
    fn encoded_cwd_replaces_slashes_and_dots_with_dashes() {
        let cwd = Path::new("/tmp/.tmpOH7bq5/repo");
        assert_eq!(encoded_cwd(cwd), "-tmp--tmpOH7bq5-repo");
    }

    #[tokio::test]
    async fn claude_new_errors_when_both_auth_envs_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let working_dir = tmp.path().to_path_buf();
        let env: HashMap<String, String> = HashMap::new();
        let result = Claude::new(None, working_dir, home, env, None, Duration::from_secs(60)).await;
        let err = match result {
            Ok(_) => panic!("expected Err"),
            Err(e) => e,
        };
        let err_str = err.to_string();
        assert!(
            err_str.contains(ENV_ANTHROPIC_API_KEY)
                || err_str.contains(ENV_CLAUDE_CODE_OAUTH_TOKEN),
            "error should mention one of the auth envs; got: {err_str}"
        );
    }

    #[tokio::test]
    async fn claude_new_succeeds_with_anthropic_api_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert(ENV_ANTHROPIC_API_KEY.to_string(), "sk-test".to_string());
        let result = Claude::new(
            None,
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            env,
            None,
            Duration::from_secs(60),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn claude_new_succeeds_with_oauth_token() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert(
            ENV_CLAUDE_CODE_OAUTH_TOKEN.to_string(),
            "tok-test".to_string(),
        );
        let result = Claude::new(
            None,
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            env,
            None,
            Duration::from_secs(60),
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn install_claude_transcript_file_returns_session_id_from_first_line() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.jsonl");
        std::fs::write(
            &src,
            br#"{"session_id":"abc-from-file","type":"summary"}
{"type":"user"}
"#,
        )
        .unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&cwd).unwrap();
        let uuid = install_claude_transcript_file(&home, &cwd, &src).unwrap();
        assert_eq!(uuid, "abc-from-file");
        let installed = transcript_path(&home, &cwd, &uuid);
        assert!(
            installed.exists(),
            "transcript should be installed at {installed:?}"
        );
    }

    async fn claude_for_test(home: &Path, cwd: &Path) -> Claude {
        let mut env = HashMap::new();
        env.insert(ENV_ANTHROPIC_API_KEY.to_string(), "sk-test".to_string());
        Claude::new(
            None,
            cwd.to_path_buf(),
            home.to_path_buf(),
            env,
            None,
            Duration::from_secs(60),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn try_materialize_round_trips_session_state_payload_v1() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let claude = claude_for_test(&home, &cwd).await;

        // The exact bytes the suspend-side uploader (`build_session_state_payload`
        // in relay_adapter.rs) produces for a running Claude session.
        let transcript = b"{\"session_id\":\"abc-uuid\",\"type\":\"summary\"}\n".to_vec();
        let payload = hydra_common::api::v1::conversations::SessionStatePayload::V1 {
            session_id: "abc-uuid".to_string(),
            transcript: Some(transcript.clone()),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();

        let native = claude.try_materialize(&bytes).expect("materialize ok");
        match native {
            NativeResume::Claude(ClaudeResume::SessionId(uuid)) => {
                assert_eq!(uuid, "abc-uuid");
            }
            other => panic!("expected NativeResume::Claude(SessionId), got {other:?}"),
        }

        // The transcript must have been installed at Claude's expected path.
        let installed = transcript_path(&home, &cwd, "abc-uuid");
        assert!(
            installed.exists(),
            "transcript should be installed at {installed:?}"
        );
        let on_disk = std::fs::read(&installed).unwrap();
        assert_eq!(on_disk, transcript);
    }

    #[tokio::test]
    async fn try_materialize_without_transcript_returns_missing_transcript_error() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let claude = claude_for_test(&home, &cwd).await;

        let payload = hydra_common::api::v1::conversations::SessionStatePayload::V1 {
            session_id: "abc-uuid".to_string(),
            transcript: None,
        };
        let bytes = serde_json::to_vec(&payload).unwrap();

        let result = claude.try_materialize(&bytes);
        assert!(matches!(result, Err(MaterializeError::MissingTranscript)));
        // No transcript was provided so nothing should be installed on disk.
        let installed = transcript_path(&home, &cwd, "abc-uuid");
        assert!(!installed.exists());
    }

    #[tokio::test]
    async fn try_materialize_returns_wrong_format_for_garbage_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let claude = claude_for_test(&home, &cwd).await;

        let result = claude.try_materialize(b"not json at all");
        assert!(matches!(result, Err(MaterializeError::WrongFormat)));

        // Also: well-formed JSON that isn't a SessionStatePayload.
        let result = claude.try_materialize(b"{\"unrelated\":42}");
        assert!(matches!(result, Err(MaterializeError::WrongFormat)));
    }

    #[test]
    fn parse_claude_events_assistant_with_tool_use_emits_tool_use() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool_1","name":"Bash","input":{"command":"ls -la","description":"List files"}}]}}"#;
        let events = parse_claude_events(line);
        let tool_use = events
            .iter()
            .find_map(|e| match e {
                ClaudeEvent::ToolUse { tool_name, payload } => Some((tool_name, payload)),
                _ => None,
            })
            .expect("expected a ToolUse event");
        assert_eq!(tool_use.0, "Bash");
        assert_eq!(
            tool_use.1.get("command").and_then(|v| v.as_str()),
            Some("ls -la")
        );
        assert_eq!(
            tool_use.1.get("description").and_then(|v| v.as_str()),
            Some("List files")
        );
    }

    #[test]
    fn parse_claude_events_tool_use_missing_name_falls_back_to_unknown_tool() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","input":{"x":1}}]}}"#;
        let events = parse_claude_events(line);
        let tool_use = events
            .iter()
            .find_map(|e| match e {
                ClaudeEvent::ToolUse { tool_name, payload } => Some((tool_name, payload)),
                _ => None,
            })
            .expect("expected a ToolUse event");
        assert_eq!(tool_use.0, "unknown_tool");
        assert_eq!(tool_use.1.get("x").and_then(|v| v.as_i64()), Some(1));
    }

    #[test]
    fn parse_claude_events_assistant_with_text_and_tool_use_emits_both() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/x"}}]}}"#;
        let events = parse_claude_events(line);
        assert!(events
            .iter()
            .any(|e| matches!(e, ClaudeEvent::Assistant { text } if text == "hi")));
        assert!(events
            .iter()
            .any(|e| matches!(e, ClaudeEvent::ToolUse { tool_name, .. } if tool_name == "Read")));
    }
}
