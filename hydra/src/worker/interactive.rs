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
/// When `conversation_resume_from` is `Some(n)`, the worker sends it as
/// `resume_from_event_index` in the WorkerConnect handshake so the server
/// responds with a catch-up that includes `session_state` for restoring the
/// prior Claude session via `claude --resume <session_id>`.
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

    // Send WorkerConnect handshake. If this is a resume, include the event
    // index so the server skips replayed events and ships session_state.
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

    // The idle-timeout upload writes the Claude session_id as UTF-8 bytes, so
    // parse it back the same way. Empty strings are treated as no session.
    let resume_session_id = parse_session_state(catch_up.session_state.as_deref());

    if let Some(ref sid) = resume_session_id {
        println!("Resuming Claude session: {sid}");
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

    // Feed catch-up user messages to Claude's stdin.
    for event in &catch_up.events {
        if let ConversationEvent::UserMessage { content, .. } = event {
            let input_line = build_claude_input(content);
            claude_stdin
                .write_all(input_line.as_bytes())
                .await
                .context("failed to write catch-up message to claude stdin")?;
            claude_stdin
                .flush()
                .await
                .context("failed to flush catch-up message to claude stdin")?;
            println!("Fed catch-up user message to Claude stdin");
        }
    }

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

                // Send Suspending event.
                let suspending_event = ConversationEvent::Suspending {
                    reason: "idle_timeout".to_string(),
                    timestamp: Utc::now(),
                };
                let suspending_msg = WorkerMessage::Event { event: suspending_event };
                if let Ok(json) = serde_json::to_string(&suspending_msg) {
                    let _ = ws_sender.send(tungstenite::Message::Text(json)).await;
                }

                // Upload session state if we have a Claude session_id.
                if let Some(ref sid) = *claude_session_id {
                    let state_upload = WorkerMessage::SessionStateUpload {
                        data: sid.as_bytes().to_vec(),
                    };
                    if let Ok(json) = serde_json::to_string(&state_upload) {
                        let _ = ws_sender.send(tungstenite::Message::Text(json)).await;
                    }
                    println!("Uploaded session state for resumption");
                }

                idle_suspended = true;
                break;
            }
        }
    }

    Ok(idle_suspended)
}

/// Parse the catch-up `session_state` blob into a Claude session_id. The
/// idle-timeout upload stores the session_id as UTF-8 bytes; empty or invalid
/// payloads mean "no resume available."
fn parse_session_state(session_state: Option<&[u8]>) -> Option<String> {
    session_state
        .map(|data| std::str::from_utf8(data).ok().map(|s| s.to_string()))?
        .filter(|s| !s.is_empty())
}

/// Build the Claude CLI argument list. When `resume_session_id` is provided,
/// `--resume <id>` is appended so Claude restores the prior conversation.
fn build_claude_args(model: Option<&str>, resume_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--verbose".to_string(),
    ];
    if let Some(sid) = resume_session_id {
        args.push("--resume".to_string());
        args.push(sid.to_string());
    }
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
    fn parse_session_state_returns_utf8_session_id() {
        let bytes = b"abc-123-def";
        assert_eq!(
            parse_session_state(Some(bytes)),
            Some("abc-123-def".to_string())
        );
    }

    #[test]
    fn parse_session_state_returns_none_for_none_input() {
        assert_eq!(parse_session_state(None), None);
    }

    #[test]
    fn parse_session_state_returns_none_for_empty_payload() {
        assert_eq!(parse_session_state(Some(&[])), None);
    }

    #[test]
    fn build_claude_args_includes_resume_when_session_id_present() {
        let args = build_claude_args(None, Some("sess-xyz"));
        assert!(args.iter().any(|a| a == "--resume"));
        assert!(args.iter().any(|a| a == "sess-xyz"));
    }

    #[test]
    fn build_claude_args_omits_resume_when_no_session_id() {
        let args = build_claude_args(None, None);
        assert!(!args.iter().any(|a| a == "--resume"));
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
    async fn test_session_state_upload_sent_on_timeout() {
        tokio::time::pause();

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // Write a session_id line to stdout, then keep it open (pending reads).
        let (stdout_read, mut stdout_write) = tokio::io::duplex(4096);
        let session_line =
            r#"{"type":"assistant","session_id":"test-session-123","message":{"content":[]}}"#;
        stdout_write
            .write_all(format!("{session_line}\n").as_bytes())
            .await
            .unwrap();
        // Don't drop stdout_write — keep it alive so further reads are pending.

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
            2,
            "expected Suspending + SessionStateUpload"
        );

        // First message: Suspending event.
        match parse_worker_message(&messages[0]) {
            WorkerMessage::Event {
                event: ConversationEvent::Suspending { reason, .. },
            } => {
                assert_eq!(reason, "idle_timeout");
            }
            other => panic!("expected Suspending event, got {other:?}"),
        }

        // Second message: SessionStateUpload with session_id bytes.
        match parse_worker_message(&messages[1]) {
            WorkerMessage::SessionStateUpload { data } => {
                assert_eq!(data, b"test-session-123");
            }
            other => panic!("expected SessionStateUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_session_state_upload_without_session_id() {
        tokio::time::pause();

        let (ws_tx, mut ws_rx) = futures::channel::mpsc::unbounded();
        let mut ws_sender = ws_tx;
        let mut ws_receiver =
            futures::stream::pending::<Result<tungstenite::Message, tungstenite::Error>>();

        // No session_id emitted — stdout stays pending.
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

        assert!(result, "should idle-suspend");
        assert!(session_id.is_none());

        drop(ws_sender);
        let messages = collect_ws_messages(&mut ws_rx);
        assert_eq!(messages.len(), 1, "only Suspending, no SessionStateUpload");
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
}
