use std::{collections::HashMap, process::Stdio, time::Duration};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{Sink, SinkExt, Stream, StreamExt};
use hydra_common::{
    api::v1::conversations::{ConversationEvent, ServerMessage, WorkerConnect, WorkerMessage},
    constants::{ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN},
    SessionId,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::Command,
};
use tokio_tungstenite::tungstenite;

use crate::claude_formatter::StreamFormatter;
use crate::client::RelayWebSocket;

/// Run an interactive Claude session, bridging between a relay WebSocket and
/// Claude's stdin/stdout via `--input-format stream-json --output-format stream-json`.
///
/// When `conversation_resume_from` is `Some(n)`, the worker is starting in a
/// brand-new container after the prior session was suspended or closed. The
/// server replays the full conversation event log in the catch-up, and the
/// worker reconstructs context by feeding a "primer" message wrapping the
/// transcript into Claude's stdin before forwarding any pending user input.
///
/// We do **not** use `claude --resume <session_id>` to restore prior context:
/// that command reads a local JSONL transcript from Claude's per-host state
/// directory, which does not exist inside a fresh hydra worker container.
pub async fn run_interactive(
    ws_stream: RelayWebSocket,
    session_id: &SessionId,
    model: Option<&str>,
    env: &HashMap<String, String>,
    working_dir: &std::path::Path,
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

    println!(
        "Catch-up received: {} events to replay",
        catch_up.events.len()
    );

    // Spawn Claude in long-lived interactive mode.
    let claude_args = build_claude_args(model);

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

    // Reconstruct context from the catch-up event log.
    feed_catch_up(&mut claude_stdin, &catch_up.events).await?;

    // Set up stdout reader with StreamFormatter.
    let mut stdout_reader = BufReader::new(claude_stdout);

    let mut claude_session_id: Option<String> = None;
    let _ = &session_id; // used for logging context

    // Relay loop: bidirectional message forwarding.
    let idle_suspended = relay_loop(
        &mut ws_sender,
        &mut ws_receiver,
        &mut claude_stdin,
        &mut stdout_reader,
        idle_timeout,
        &mut claude_session_id,
    )
    .await?;

    // Clean shutdown: terminate Claude process.
    if idle_suspended {
        println!("Shutting down interactive session (idle timeout)");
    } else {
        println!("Shutting down interactive session");
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

/// Feed catch-up events to Claude's stdin. If the event log contains any
/// `AssistantMessage` events, the prior history up through the most recent
/// reply is wrapped into a single "primer" user message and sent first so
/// Claude treats it as historical context. Any trailing `UserMessage` events
/// (sent before the new worker connected) are then forwarded normally so
/// Claude can respond to them.
async fn feed_catch_up<W: AsyncWrite + Unpin>(
    claude_stdin: &mut W,
    events: &[ConversationEvent],
) -> Result<()> {
    let (past_context, pending_user_messages) = partition_events(events);

    if !past_context.is_empty() {
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
        let input_line = build_claude_input(content);
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
                transcript.push_str(content);
                transcript.push('\n');
            }
            ConversationEvent::AssistantMessage { content, .. } => {
                transcript.push_str("Assistant: ");
                transcript.push_str(content);
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
</prior-conversation>\n\
\n\
Acknowledge in one short sentence that you've received this context, then wait for the next user message."
    )
}

/// Core relay loop: bidirectional message forwarding between WebSocket and Claude
/// stdin/stdout. Returns `Ok(true)` if idle-suspended, `Ok(false)` if exited normally.
async fn relay_loop<St, Si, W, R>(
    ws_sender: &mut Si,
    ws_receiver: &mut St,
    claude_stdin: &mut W,
    stdout_reader: &mut BufReader<R>,
    idle_timeout: Duration,
    claude_session_id: &mut Option<String>,
) -> Result<bool>
where
    St: Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
    Si: Sink<tungstenite::Message> + Unpin,
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let mut stdout_line = String::new();
    let mut formatter = StreamFormatter::new();
    let mut idle_suspended = false;

    let idle_deadline = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_deadline);

    loop {
        tokio::select! {
            // Claude stdout -> Server: parse stream-json, send assistant messages.
            read_result = stdout_reader.read_line(&mut stdout_line) => {
                match read_result {
                    Ok(0) => {
                        println!("Claude stdout EOF");
                        break;
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
                                    break;
                                }
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

            // Server -> Claude: receive user messages from WebSocket.
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(ServerMessage::Event { event }) => {
                                if let ConversationEvent::UserMessage { content, .. } = event {
                                    // Reset idle timer on user input.
                                    idle_deadline.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                                    let input_line = build_claude_input(&content);
                                    if claude_stdin
                                        .write_all(input_line.as_bytes())
                                        .await
                                        .is_err()
                                    {
                                        eprintln!("Failed to write to Claude stdin (process may have exited)");
                                        break;
                                    }
                                    if claude_stdin.flush().await.is_err() {
                                        eprintln!("Failed to flush Claude stdin");
                                        break;
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
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ignore binary, pong, etc.
                    }
                    Some(Err(err)) => {
                        eprintln!("WebSocket error: {err}");
                        break;
                    }
                }
            }

            // Idle timeout: suspend the session when no user input is received.
            _ = &mut idle_deadline => {
                println!("Idle timeout reached ({idle_timeout:?}), suspending session");

                // Send Suspending event so the server transitions the
                // conversation to Idle. We no longer upload a session_state
                // blob: the server-stored event log is the source of truth
                // and a fresh worker rebuilds context via the primer flow.
                let suspending_event = ConversationEvent::Suspending {
                    reason: "idle_timeout".to_string(),
                    timestamp: Utc::now(),
                };
                let suspending_msg = WorkerMessage::Event { event: suspending_event };
                if let Ok(json) = serde_json::to_string(&suspending_msg) {
                    let _ = ws_sender.send(tungstenite::Message::Text(json)).await;
                }

                idle_suspended = true;
                break;
            }
        }
    }

    Ok(idle_suspended)
}

/// Build the Claude CLI argument list.
fn build_claude_args(model: Option<&str>) -> Vec<String> {
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
    fn build_claude_args_omits_resume_argument() {
        let args = build_claude_args(None);
        assert!(
            !args.iter().any(|a| a == "--resume"),
            "build_claude_args must never emit --resume; resume is handled via the context primer"
        );
    }

    #[test]
    fn build_claude_args_includes_model_when_set() {
        let args = build_claude_args(Some("opus"));
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
            primer.contains("Acknowledge"),
            "primer must instruct the agent how to respond"
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

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Keep _stdout_write alive so reads block (pending), not EOF.
        let (stdout_read, _stdout_write) = tokio::io::duplex(1024);
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut claude_stdin = tokio::io::sink();
        let mut session_id = None;

        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
        )
        .await
        .unwrap();

        assert!(result, "should return true for idle suspended");

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
    async fn test_idle_timeout_resets_on_user_message() {
        tokio::time::pause();

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

        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
        )
        .await
        .unwrap();

        assert!(result, "should eventually idle-suspend");

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
    async fn test_idle_timeout_does_not_emit_session_state_upload() {
        tokio::time::pause();

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Write a session_id line to stdout, then keep it open (pending reads).
        // Even though the worker captures the session_id, we no longer upload
        // it — the server-stored event log is the source of truth.
        let (stdout_read, mut stdout_write) = tokio::io::duplex(4096);
        let session_line =
            r#"{"type":"assistant","session_id":"test-session-123","message":{"content":[]}}"#;
        stdout_write
            .write_all(format!("{session_line}\n").as_bytes())
            .await
            .unwrap();

        let mut claude_stdin = tokio::io::sink();
        let mut stdout_reader = BufReader::new(stdout_read);
        let mut session_id = None;

        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
        )
        .await
        .unwrap();

        assert!(result, "should idle-suspend");
        assert_eq!(session_id.as_deref(), Some("test-session-123"));

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert_eq!(
            messages.len(),
            1,
            "expected only Suspending; SessionStateUpload must not be sent"
        );

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
    async fn test_claude_stdout_eof_exits_without_suspending() {
        tokio::time::pause();

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

        let result = relay_loop(
            &mut ws_sender,
            &mut ws_receiver,
            &mut claude_stdin,
            &mut stdout_reader,
            Duration::from_millis(50),
            &mut session_id,
        )
        .await
        .unwrap();

        assert!(!result, "should return false (not idle suspended)");

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert!(messages.is_empty(), "no Suspending event should be sent");
    }

    #[tokio::test]
    async fn test_feed_catch_up_sends_primer_then_pending() {
        // Simulate a catch-up with prior history (msg1, reply1, msg2, reply2)
        // plus a pending user message (msg3) that the prior worker did not
        // reply to. The first stdin line must be the primer wrapping the
        // prior transcript; the second must be msg3.
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
        ];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(8192);
        let mut claude_stdin = stdin_write;

        feed_catch_up(&mut claude_stdin, &events).await.unwrap();
        // Drop the writer so the reader sees EOF after the buffered bytes.
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
        // The primer must NOT contain the pending msg3 (it is sent separately).
        assert!(!primer_content.contains("User: msg3"));

        let mut second_line = String::new();
        reader.read_line(&mut second_line).await.unwrap();
        let second: serde_json::Value = serde_json::from_str(&second_line).unwrap();
        assert_eq!(second["type"], "user");
        assert_eq!(second["message"]["content"], "msg3");

        // No further lines.
        let mut trailing = String::new();
        let n = reader.read_line(&mut trailing).await.unwrap();
        assert_eq!(n, 0, "no trailing input expected, got {trailing:?}");
    }

    #[tokio::test]
    async fn test_feed_catch_up_no_primer_when_no_assistant_history() {
        // A fresh conversation with one pending user message and no
        // AssistantMessage events yet should NOT emit a primer.
        let events = vec![user_msg("hello")];

        let (mut stdin_read, stdin_write) = tokio::io::duplex(4096);
        let mut claude_stdin = stdin_write;

        feed_catch_up(&mut claude_stdin, &events).await.unwrap();
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

        feed_catch_up(&mut claude_stdin, &[]).await.unwrap();
        drop(claude_stdin);

        let mut reader = BufReader::new(&mut stdin_read);
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.unwrap();
        assert_eq!(n, 0, "no input expected for empty catch-up");
    }
}
