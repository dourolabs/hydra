//! Translates between the relay WebSocket protocol and the generic
//! `WorkerInputMessage` / `WorkerEvent` channels consumed by
//! [`crate::worker::model_selector::ModelSelector`].
//!
//! See `designs/worker-model-commands-refactor.md` §2.3 for the design. This
//! module owns:
//! * the `WorkerConnect` handshake and catch-up drain,
//! * the bidirectional pump between the WebSocket and the generic channels,
//! * idle-timeout / SIGTERM detection and the suspend-emission flow.
//!
//! It does **not** know about Claude-native or Codex-native types — those
//! translations live inside `ModelSelector` (per design §3).

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{Sink, SinkExt, StreamExt};
use hydra_common::{
    api::v1::{
        conversations::{ServerMessage, SessionStatePayload, WorkerConnect, WorkerMessage},
        sessions::SessionEvent,
    },
    SessionId,
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};

use crate::client::RelayWebSocket;
use crate::worker::claude::transcript_path;
use crate::worker::report::{SessionResume, WorkerEvent, WorkerInputMessage};

/// Handles returned by [`spawn_relay_adapter`].
pub struct RelayAdapter {
    /// Caller-side input receiver. Carries `WorkerInputMessage`s produced from
    /// catch-up and relay `UserMessage` events. The caller should pass this
    /// into `ModelSelector::run_interactive`.
    pub input_rx: mpsc::Receiver<WorkerInputMessage>,
    /// Caller-side output sender. The caller passes this into
    /// `ModelSelector::run_interactive`; the model emits `WorkerEvent`s on it
    /// which the relay adapter consumes and forwards onto the WebSocket.
    pub output_tx: mpsc::Sender<WorkerEvent>,
    /// Join handle for the relay-pump task. The caller drops it without
    /// awaiting in normal flow; the task ends when both the WebSocket and the
    /// output channel close.
    pub pump: tokio::task::JoinHandle<()>,
    /// Resolves once during catch-up with the `SessionResume` the caller
    /// should pass to `ModelSelector::run_interactive`. Resolves to `None` if
    /// the catch-up does not carry a usable session state.
    pub initial_resume: oneshot::Receiver<Option<SessionResume>>,
}

/// Spawn the relay adapter. Returns immediately; the WS handshake/catch-up
/// proceed inside the spawned pump task. The caller should `.await` the
/// `initial_resume` oneshot before invoking `ModelSelector::run_interactive`.
pub fn spawn_relay_adapter(
    ws: RelayWebSocket,
    session_id: &SessionId,
    conversation_resume_from: Option<usize>,
    prompt: &str,
    home_dir: PathBuf,
    working_dir: PathBuf,
    idle_timeout: Duration,
) -> RelayAdapter {
    let (input_tx, input_rx) = mpsc::channel::<WorkerInputMessage>(32);
    let (output_tx, output_rx) = mpsc::channel::<WorkerEvent>(32);
    let (initial_resume_tx, initial_resume_rx) = oneshot::channel::<Option<SessionResume>>();
    let session_id = session_id.clone();
    let prompt = prompt.to_string();

    let pump = tokio::spawn(async move {
        if let Err(err) = run_pump(
            ws,
            &session_id,
            conversation_resume_from,
            &prompt,
            home_dir,
            working_dir,
            idle_timeout,
            input_tx,
            output_rx,
            initial_resume_tx,
        )
        .await
        {
            error!(error = %err, "relay_adapter pump exited with error");
        }
    });

    RelayAdapter {
        input_rx,
        output_tx,
        pump,
        initial_resume: initial_resume_rx,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_pump(
    ws: RelayWebSocket,
    session_id: &SessionId,
    conversation_resume_from: Option<usize>,
    prompt: &str,
    home_dir: PathBuf,
    working_dir: PathBuf,
    idle_timeout: Duration,
    input_tx: mpsc::Sender<WorkerInputMessage>,
    mut output_rx: mpsc::Receiver<WorkerEvent>,
    initial_resume_tx: oneshot::Sender<Option<SessionResume>>,
) -> Result<()> {
    let (mut ws_sender, mut ws_receiver) = ws.split();

    println!("WebSocket connected, sending handshake");

    let handshake = WorkerConnect::Fresh {
        resume_from_event_index: conversation_resume_from,
    };
    let handshake_json =
        serde_json::to_string(&handshake).context("failed to serialize WorkerConnect")?;
    ws_sender
        .send(tungstenite::Message::Text(handshake_json))
        .await
        .context("failed to send WorkerConnect handshake")?;

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

    info!(
        %session_id,
        events = catch_up.events.len(),
        "catch_up_received"
    );

    // Catch-up no longer carries session_state (see i-xwmoxzhe — shipping it
    // pushed long conversations' catch-up frames past the WebSocket 16 MiB
    // cap and silently killed every resume). The worker hasn't read those
    // bytes since the transcript-based `claude --resume <UUID>` path was
    // removed; we rebuild context from `catch_up.events` via the
    // primer-merge path below. The suspend-side `SessionStateUpload` upload
    // is unchanged so we can revive catch-up-side delivery later without
    // re-implementing the writer.
    let _ = initial_resume_tx.send(None);

    let mut prompt_prepend = PromptPrepend::new(prompt, &catch_up.events);

    // Feed catch-up: build a primer wrapping the prior transcript and merge it
    // with the first pending `UserMessage` into a single `WorkerInputMessage`,
    // so the model sees prior context and the actual question as one turn.
    feed_catch_up_to_channel(&input_tx, &catch_up.events, prompt, &mut prompt_prepend).await?;

    // Track the model session id reported via WorkerEvent::SessionInit so the
    // suspend-upload code can find the transcript on disk.
    let mut model_session_id: Option<String> = None;

    let idle_deadline = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_deadline);

    #[cfg(unix)]
    let mut sigterm_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;

    loop {
        tokio::select! {
            event = output_rx.recv() => {
                match event {
                    Some(WorkerEvent::AssistantText { text }) => {
                        if !text.is_empty() {
                            let session_event = SessionEvent::AssistantMessage {
                                content: text,
                                timestamp: Utc::now(),
                            };
                            let msg = WorkerMessage::Event { event: session_event };
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
                    Some(WorkerEvent::SessionInit { model_session_id: sid }) => {
                        info!(%session_id, model_session_id = %sid, "session_init_observed");
                        model_session_id = Some(sid);
                    }
                    Some(WorkerEvent::Usage { .. }) => {
                        // Token usage is tracked by the model wrapper and
                        // surfaced in `RunReport`; no relay-side action.
                    }
                    Some(WorkerEvent::ToolUse { .. }) => {
                        // ToolUse is consumed by the PR-3 dispatch layer
                        // (forwarded as `SessionEvent::ToolUse`); the
                        // legacy relay_adapter has no story for it and is
                        // being retired in PR-3.
                        tracing::trace!("relay_adapter: WorkerEvent::ToolUse ignored");
                    }
                    Some(WorkerEvent::Raw { .. }) => {
                        tracing::trace!("relay_adapter: WorkerEvent::Raw ignored");
                    }
                    None => {
                        // Model output stream closed — the run is done. Exit
                        // the pump; the caller will see the WS close on next
                        // send.
                        println!("Model output channel closed");
                        break;
                    }
                }
            }

            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(ServerMessage::Event { event }) => {
                                if let SessionEvent::UserMessage { content, .. } = event {
                                    idle_deadline
                                        .as_mut()
                                        .reset(tokio::time::Instant::now() + idle_timeout);
                                    let to_send = prompt_prepend.apply(&content);
                                    if input_tx
                                        .send(WorkerInputMessage { content: to_send })
                                        .await
                                        .is_err()
                                    {
                                        eprintln!("Model input channel closed; cannot forward user message");
                                        break;
                                    }
                                    println!("Forwarded user message to model input channel");
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
                        let _ = ws_sender.send(tungstenite::Message::Pong(data)).await;
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        println!("WebSocket closed by server");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        eprintln!("WebSocket error: {err}");
                        break;
                    }
                }
            }

            _ = &mut idle_deadline => {
                println!("Idle timeout reached ({idle_timeout:?}), suspending session");
                emit_suspend(
                    &mut ws_sender,
                    "idle_timeout",
                    model_session_id.as_deref(),
                    &home_dir,
                    &working_dir,
                )
                .await;
                break;
            }

            _ = await_sigterm(
                #[cfg(unix)]
                &mut sigterm_signal,
            ) => {
                println!("SIGTERM received, suspending session");
                emit_suspend(
                    &mut ws_sender,
                    "sigterm",
                    model_session_id.as_deref(),
                    &home_dir,
                    &working_dir,
                )
                .await;
                break;
            }
        }
    }

    // Drop the input sender to signal "no more input" to the model wrapper.
    drop(input_tx);

    let _ = ws_sender.send(tungstenite::Message::Close(None)).await;
    Ok(())
}

/// Feed catch-up events into the generic input channel.
///
/// We build a context primer wrapping the prior conversation transcript and
/// merge it with the first pending `UserMessage` into a single
/// `WorkerInputMessage`, so the model sees prior context and the actual
/// question as one turn. Without this merge, sending the primer as its own
/// input elicits a content-free meta-ack (e.g. "Ready when you are.")
/// because the primer alone has no question to answer.
///
/// If there are no prior events the primer step is skipped. If there are no
/// pending user messages (e.g. `/resume` without an attached message), the
/// primer is deferred via [`PromptPrepend::defer_primer`] and prepended to
/// the first relay-loop user message instead.
async fn feed_catch_up_to_channel(
    input_tx: &mpsc::Sender<WorkerInputMessage>,
    events: &[SessionEvent],
    prompt: &str,
    prompt_prepend: &mut PromptPrepend,
) -> Result<()> {
    let (past_context, pending_user_messages) = partition_events(events);

    let primer = if !past_context.is_empty() {
        Some(build_context_primer(prompt, &past_context))
    } else {
        None
    };

    let mut pending_iter = pending_user_messages.into_iter();

    if let Some(primer) = primer {
        match pending_iter.next() {
            Some(first) => {
                let combined = format!("{primer}\n\n{first}");
                input_tx
                    .send(WorkerInputMessage { content: combined })
                    .await
                    .context(
                        "failed to send merged primer + first pending user message \
                         to model input channel",
                    )?;
                println!(
                    "Sent merged primer ({} prior events) + first pending user message to model input channel",
                    past_context.len()
                );
            }
            None => {
                prompt_prepend.defer_primer(primer);
                println!(
                    "Deferred primer ({} prior events) until first relay user message",
                    past_context.len()
                );
            }
        }
    }

    for content in pending_iter {
        let to_send = prompt_prepend.apply(content);
        input_tx
            .send(WorkerInputMessage { content: to_send })
            .await
            .context("failed to send catch-up user message to model input channel")?;
        println!("Sent catch-up user message to model input channel");
    }

    Ok(())
}

/// Tracks one-shot prepends applied to the next user message reaching the
/// model input channel.
///
/// Two kinds of prepends are possible, but never both on the same input:
/// * `agent_prompt_pending` — the `<agent-prompt>` wrapper applied to the
///   first user message in a fresh conversation.
/// * `primer_pending` — a prior-context primer queued by
///   `feed_catch_up_to_channel` when we resumed without a usable
///   transcript and the catch-up carried no trailing pending user
///   message. The primer already embeds the `<agent-prompt>` wrapper, so
///   queuing one suppresses the separate agent-prompt prepend.
#[derive(Debug)]
struct PromptPrepend {
    prompt: String,
    agent_prompt_pending: bool,
    primer_pending: Option<String>,
}

impl PromptPrepend {
    fn new(prompt: &str, catch_up_events: &[SessionEvent]) -> Self {
        let has_assistant = catch_up_events
            .iter()
            .any(|e| matches!(e, SessionEvent::AssistantMessage { .. }));
        let agent_prompt_pending = !prompt.is_empty() && !has_assistant;
        Self {
            prompt: prompt.to_string(),
            agent_prompt_pending,
            primer_pending: None,
        }
    }

    /// Defer a primer until the next user message arrives. Because the
    /// primer already embeds `<agent-prompt>`, this also suppresses the
    /// agent-prompt prepend so the wrapper isn't applied twice.
    fn defer_primer(&mut self, primer: String) {
        self.primer_pending = Some(primer);
        self.agent_prompt_pending = false;
    }

    fn apply(&mut self, content: &str) -> String {
        if let Some(primer) = self.primer_pending.take() {
            self.agent_prompt_pending = false;
            format!("{primer}\n\n{content}")
        } else if self.agent_prompt_pending {
            self.agent_prompt_pending = false;
            format!("{}\n\n{content}", self.prompt)
        } else {
            content.to_string()
        }
    }
}

/// Partition a catch-up event log into past context (everything up to and
/// including the last assistant message) and pending user messages
/// (everything after the last assistant message, or — if no assistant message
/// is present — every `UserMessage` in the log).
fn partition_events(events: &[SessionEvent]) -> (Vec<&SessionEvent>, Vec<&str>) {
    let last_assistant_idx = events
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e, SessionEvent::AssistantMessage { .. }))
        .map(|(i, _)| i);

    let mut past_context: Vec<&SessionEvent> = Vec::new();
    let mut pending: Vec<&str> = Vec::new();

    match last_assistant_idx {
        Some(idx) => {
            for event in &events[..=idx] {
                if matches!(
                    event,
                    SessionEvent::UserMessage { .. } | SessionEvent::AssistantMessage { .. }
                ) {
                    past_context.push(event);
                }
            }
            for event in &events[idx + 1..] {
                if let SessionEvent::UserMessage { content, .. } = event {
                    pending.push(content.as_str());
                }
            }
        }
        None => {
            for event in events {
                if let SessionEvent::UserMessage { content, .. } = event {
                    pending.push(content.as_str());
                }
            }
        }
    }

    (past_context, pending)
}

/// Build a single primer message that wraps the prior transcript so the model
/// can use it as historical context.
fn build_context_primer(prompt: &str, past_context: &[&SessionEvent]) -> String {
    let mut transcript = String::new();
    for event in past_context {
        match event {
            SessionEvent::UserMessage { content, .. } => {
                transcript.push_str("User: ");
                transcript.push_str(&escape_wrapper_close(content));
                transcript.push('\n');
            }
            SessionEvent::AssistantMessage { content, .. } => {
                transcript.push_str("Assistant: ");
                transcript.push_str(&escape_wrapper_close(content));
                transcript.push('\n');
            }
            _ => {}
        }
    }

    let prior = format!(
        "<prior-conversation>\n\
The user and I had this prior conversation. The conversation was suspended or closed and is now being resumed. \
Treat this as historical context only — do not re-execute or repeat any actions described. \
Respond only to the next user message after this block.\n\
\n\
{transcript}\
</prior-conversation>"
    );

    if prompt.is_empty() {
        prior
    } else {
        let escaped = escape_agent_prompt_close(prompt);
        format!("<agent-prompt>\n{escaped}\n</agent-prompt>\n\n{prior}")
    }
}

fn escape_wrapper_close(content: &str) -> String {
    content.replace("</prior-conversation>", "</prior-conversation\u{200B}>")
}

fn escape_agent_prompt_close(content: &str) -> String {
    content.replace("</agent-prompt>", "</agent-prompt\u{200B}>")
}

/// Suspend-emission helper: writes a `Suspending` event followed by a
/// best-effort `SessionStateUpload` carrying the transcript file.
async fn emit_suspend<Si>(
    ws_sender: &mut Si,
    reason: &str,
    model_session_id: Option<&str>,
    home_dir: &Path,
    working_dir: &Path,
) where
    Si: Sink<tungstenite::Message> + Unpin,
{
    info!(
        reason,
        has_model_session_id = model_session_id.is_some(),
        "emit_suspend entry"
    );
    let suspending_event = SessionEvent::Suspending {
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

    let Some(sid) = model_session_id else {
        warn!(
            reason,
            "emit_suspend transcript_upload_skipped — no model session id observed"
        );
        return;
    };

    let payload = build_session_state_payload(home_dir, working_dir, sid).await;
    match send_session_state_upload(ws_sender, &payload).await {
        Ok(()) => info!(reason, model_session_id = sid, "emit_suspend upload_ok"),
        Err(err) => error!(
            reason,
            model_session_id = sid,
            error = %err,
            "emit_suspend upload_failed"
        ),
    }
}

/// Build a `SessionStatePayload` from the captured model session id and the
/// current contents of its transcript file.
async fn build_session_state_payload(
    home_dir: &Path,
    working_dir: &Path,
    session_id: &str,
) -> SessionStatePayload {
    let path = transcript_path(home_dir, working_dir, session_id);
    let transcript = match tokio::fs::read(&path).await {
        Ok(bytes) => {
            info!(
                model_session_id = session_id,
                transcript_path = %path.display(),
                bytes = bytes.len(),
                "build_session_state_payload read_ok"
            );
            Some(bytes)
        }
        Err(err) => {
            warn!(
                model_session_id = session_id,
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

/// Send a `SessionStateUpload` over the WebSocket.
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

#[cfg(unix)]
async fn await_sigterm(sig: &mut tokio::signal::unix::Signal) {
    sig.recv().await;
}

#[cfg(not(unix))]
async fn await_sigterm() {
    futures::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> SessionEvent {
        SessionEvent::UserMessage {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn assistant_msg(content: &str) -> SessionEvent {
        SessionEvent::AssistantMessage {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn partition_events_returns_past_context_and_pending_user_messages() {
        let events = vec![
            user_msg("msg1"),
            assistant_msg("reply1"),
            user_msg("msg2"),
            assistant_msg("reply2"),
            user_msg("msg3"),
        ];
        let (past, pending) = partition_events(&events);
        assert_eq!(past.len(), 4);
        assert_eq!(pending, vec!["msg3"]);
    }

    #[test]
    fn partition_events_no_assistant_treats_all_as_pending() {
        let events = vec![user_msg("a"), user_msg("b")];
        let (past, pending) = partition_events(&events);
        assert!(past.is_empty());
        assert_eq!(pending, vec!["a", "b"]);
    }

    #[test]
    fn prompt_prepend_applies_to_first_message_only() {
        let mut p = PromptPrepend::new("agent", &[]);
        let first = p.apply("hello");
        assert_eq!(first, "agent\n\nhello");
        let second = p.apply("world");
        assert_eq!(second, "world");
    }

    #[test]
    fn prompt_prepend_suppressed_when_prior_assistant_exists() {
        let events = vec![user_msg("u1"), assistant_msg("a1")];
        let mut p = PromptPrepend::new("agent", &events);
        let out = p.apply("hello");
        assert_eq!(out, "hello");
    }

    #[test]
    fn prompt_prepend_suppressed_when_prompt_empty() {
        let mut p = PromptPrepend::new("", &[]);
        let out = p.apply("hello");
        assert_eq!(out, "hello");
    }

    #[test]
    fn build_context_primer_no_prompt_emits_only_prior_block() {
        let user = user_msg("u1");
        let asst = assistant_msg("a1");
        let primer = build_context_primer("", &[&user, &asst]);
        assert!(primer.contains("<prior-conversation>"));
        assert!(!primer.contains("<agent-prompt>"));
    }

    #[test]
    fn build_context_primer_with_prompt_wraps_in_agent_prompt() {
        let user = user_msg("u1");
        let primer = build_context_primer("agent text", &[&user]);
        assert!(primer.starts_with("<agent-prompt>"));
        assert!(primer.contains("agent text"));
        assert!(primer.contains("<prior-conversation>"));
    }

    async fn drain(rx: &mut mpsc::Receiver<WorkerInputMessage>) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(msg) = rx.recv().await {
            out.push(msg.content);
        }
        out
    }

    #[tokio::test]
    async fn feed_catch_up_merges_primer_with_first_pending_user_message() -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<WorkerInputMessage>(8);
        let events = vec![
            user_msg("u1"),
            assistant_msg("a1"),
            user_msg("new question"),
        ];
        let mut prepend = PromptPrepend::new("agent", &events);
        feed_catch_up_to_channel(&tx, &events, "agent", &mut prepend).await?;
        drop(tx);

        let received = drain(&mut rx).await;
        assert_eq!(
            received.len(),
            1,
            "expected exactly one merged input, got {received:?}"
        );
        assert!(
            received[0].contains("<prior-conversation>"),
            "merged input must include the primer"
        );
        assert!(
            received[0].contains("new question"),
            "merged input must include the pending user message"
        );
        assert!(
            prepend.primer_pending.is_none(),
            "primer must not also be deferred when it was merged"
        );
        Ok(())
    }

    #[tokio::test]
    async fn feed_catch_up_with_multiple_pending_sends_merged_first_then_rest() -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<WorkerInputMessage>(8);
        let events = vec![
            user_msg("u1"),
            assistant_msg("a1"),
            user_msg("first pending"),
            user_msg("second pending"),
        ];
        let mut prepend = PromptPrepend::new("agent", &events);
        feed_catch_up_to_channel(&tx, &events, "agent", &mut prepend).await?;
        drop(tx);

        let received = drain(&mut rx).await;
        assert_eq!(received.len(), 2, "expected 2 inputs, got {received:?}");
        assert!(received[0].contains("<prior-conversation>"));
        assert!(received[0].contains("first pending"));
        assert!(
            !received[1].contains("<prior-conversation>"),
            "second pending must not carry the primer"
        );
        assert_eq!(received[1], "second pending");
        Ok(())
    }

    #[tokio::test]
    async fn feed_catch_up_with_empty_pending_defers_primer_until_first_relay_message() -> Result<()>
    {
        let (tx, mut rx) = mpsc::channel::<WorkerInputMessage>(8);
        let events = vec![user_msg("u1"), assistant_msg("a1")];
        let mut prepend = PromptPrepend::new("agent", &events);
        feed_catch_up_to_channel(&tx, &events, "agent", &mut prepend).await?;
        drop(tx);

        let received = drain(&mut rx).await;
        assert!(
            received.is_empty(),
            "no input must be sent during catch-up when pending is empty; got {received:?}"
        );

        let first = prepend.apply("relay msg");
        assert!(
            first.contains("<prior-conversation>"),
            "primer must be prepended to the first relay-loop user message"
        );
        assert!(first.contains("relay msg"));

        let second = prepend.apply("another relay msg");
        assert_eq!(
            second, "another relay msg",
            "primer must only fire on the first relay-loop user message"
        );
        Ok(())
    }

    /// Regression test for i-tayyxxxf: chat close→reopen→send. The catch-up
    /// the resumed worker receives is the cross-session SessionEvent log —
    /// it carries the full prior chat history, lifecycle markers
    /// (`Suspending` / `Resumed`), and the post-close trailing user message.
    /// `feed_catch_up_to_channel` must merge the prior history into a primer
    /// and combine it with the trailing user message into one stdin input so
    /// the model has both context and an unanswered question to respond to.
    /// Lifecycle events between the last assistant message and the trailing
    /// user message must be ignored.
    #[tokio::test]
    async fn feed_catch_up_chat_close_then_resume_merges_history_with_trailing_user_message(
    ) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<WorkerInputMessage>(8);
        // Mirrors the cross-session catch-up that `build_catch_up` returns
        // after the user closes a chat with three completed turns and sends
        // a fourth message on the resumed session.
        let suspending = SessionEvent::Suspending {
            reason: "idle_timeout".to_string(),
            timestamp: Utc::now(),
        };
        let resumed = SessionEvent::Resumed {
            from_session_id: hydra_common::SessionId::new(),
            timestamp: Utc::now(),
        };
        let events = vec![
            user_msg("My name is Alice. What's 2+2?"),
            assistant_msg("4"),
            user_msg("I'm a software engineer. What's 3+3?"),
            assistant_msg("6"),
            user_msg("I work on Rust projects. What's 4+4?"),
            assistant_msg("8"),
            suspending,
            resumed,
            user_msg("What's my name and what do I work on?"),
        ];
        let mut prepend = PromptPrepend::new("agent", &events);
        feed_catch_up_to_channel(&tx, &events, "agent", &mut prepend).await?;
        drop(tx);

        let received = drain(&mut rx).await;
        assert_eq!(
            received.len(),
            1,
            "expected exactly one merged input — the primer + trailing user \
             message — got {received:?}"
        );
        let merged = &received[0];
        assert!(
            merged.contains("<prior-conversation>"),
            "merged input must wrap the prior history in a <prior-conversation> \
             block, got: {merged}"
        );
        // Each completed turn from the prior history must appear in the primer.
        for needle in [
            "My name is Alice. What's 2+2?",
            "I'm a software engineer. What's 3+3?",
            "I work on Rust projects. What's 4+4?",
        ] {
            assert!(
                merged.contains(needle),
                "primer must carry prior user message {needle:?}, got: {merged}"
            );
        }
        for needle in ["4", "6", "8"] {
            assert!(
                merged.contains(needle),
                "primer must carry prior assistant reply {needle:?}, got: {merged}"
            );
        }
        // Lifecycle markers must not bleed into the primer text — only
        // `UserMessage` / `AssistantMessage` events are part of the prior log.
        assert!(
            !merged.contains("idle_timeout"),
            "Suspending reason must not appear in the primer, got: {merged}"
        );
        // The trailing user message after the last assistant reply must be the
        // tail of the merged input so the model sees it as the active question.
        assert!(
            merged.contains("What's my name and what do I work on?"),
            "merged input must include the trailing user message, got: {merged}"
        );
        assert!(
            prepend.primer_pending.is_none(),
            "primer must not also be deferred when it was merged"
        );
        Ok(())
    }
}
