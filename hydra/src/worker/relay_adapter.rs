//! Worker-side WebSocket adapter for the per-session events channel.
//!
//! Implements the three-phase protocol described in
//! `designs/sessions-worker-run-interface.md` §1.1:
//!
//! 1. **Context negotiation.** Worker sends [`WorkerConnect::Fresh`]; server
//!    replies with [`ServerMessage::ResumeContext`]. If the resume_blob is
//!    present and the wrapper can materialize it, we use the native handle;
//!    otherwise (or if materialization fails) the caller may fall back to
//!    [`WorkerMessage::RequestTranscript`].
//! 2. **First message.** Worker sends [`WorkerMessage::Ready`]; server
//!    replies with [`ServerMessage::FirstMessage`]. The caller concatenates
//!    `agent_prompt + "\n\n" + user_message` and uses the result as the
//!    first model input.
//! 3. **Subsequent messages.** Bidirectional pump of `SessionEvent`s.
//!
//! The pump exposed by this module owns the Phase-3 work. The
//! `ModelSelector::drive_*` callers own Phases 1 and 2 explicitly via direct
//! `WorkerSocket` send/recv calls.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{Sink, SinkExt};
use hydra_common::{
    api::v1::{
        conversations::{ServerMessage, SessionStatePayload, WorkerMessage},
        sessions::SessionEvent,
    },
    SessionId,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};

use crate::client::RelayWebSocket;
use crate::worker::claude::transcript_path;
use crate::worker::report::{WorkerEvent, WorkerInputMessage};

/// Handles returned by [`spawn_pump`].
pub struct RelayAdapter {
    /// Caller-side input receiver. Carries `WorkerInputMessage`s produced from
    /// inbound `SessionEvent::UserMessage` events the relay pumps onto this
    /// channel. The caller passes this into
    /// `ModelSelector::run_interactive_with_native`.
    pub input_rx: mpsc::Receiver<WorkerInputMessage>,
    /// Caller-side output sender. The caller passes this into
    /// `ModelSelector::run_interactive_with_native`; the model emits
    /// `WorkerEvent`s on it and the relay adapter consumes and forwards them
    /// onto the WebSocket as `SessionEvent`s.
    pub output_tx: mpsc::Sender<WorkerEvent>,
    /// Join handle for the relay-pump task.
    pub pump: tokio::task::JoinHandle<()>,
}

/// Spawn the Phase-3 bidirectional pump. The caller is responsible for
/// having already performed Phases 1 and 2 on `ws` (handshake, ResumeContext,
/// Ready/FirstMessage exchange).
#[allow(clippy::too_many_arguments)]
pub fn spawn_pump(
    ws: RelayWebSocket,
    session_id: &SessionId,
    home_dir: PathBuf,
    working_dir: PathBuf,
    idle_timeout: Duration,
) -> RelayAdapter {
    let (input_tx, input_rx) = mpsc::channel::<WorkerInputMessage>(32);
    let (output_tx, output_rx) = mpsc::channel::<WorkerEvent>(32);
    let session_id = session_id.clone();

    let pump = tokio::spawn(async move {
        if let Err(err) = run_pump(
            ws,
            &session_id,
            home_dir,
            working_dir,
            idle_timeout,
            input_tx,
            output_rx,
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
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_pump(
    ws: RelayWebSocket,
    session_id: &SessionId,
    home_dir: PathBuf,
    working_dir: PathBuf,
    idle_timeout: Duration,
    input_tx: mpsc::Sender<WorkerInputMessage>,
    mut output_rx: mpsc::Receiver<WorkerEvent>,
) -> Result<()> {
    use futures::StreamExt;
    let (mut ws_sender, mut ws_receiver) = ws.split();

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
                            if !send_worker_event(&mut ws_sender, session_event).await {
                                break;
                            }
                        }
                    }
                    Some(WorkerEvent::SessionInit { model_session_id: sid }) => {
                        info!(%session_id, model_session_id = %sid, "session_init_observed");
                        model_session_id = Some(sid);
                    }
                    Some(WorkerEvent::Usage { .. }) => {
                        // Token usage tracked by the model wrapper and
                        // surfaced via RunReport.
                    }
                    Some(WorkerEvent::ToolUse { tool_name, payload }) => {
                        let session_event = SessionEvent::ToolUse {
                            tool_name,
                            payload,
                            timestamp: Utc::now(),
                        };
                        if !send_worker_event(&mut ws_sender, session_event).await {
                            break;
                        }
                    }
                    Some(WorkerEvent::Raw { .. }) => {
                        tracing::trace!("relay_adapter: WorkerEvent::Raw ignored");
                    }
                    None => {
                        // Model output stream closed — the run is done.
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
                                    if input_tx
                                        .send(WorkerInputMessage { content })
                                        .await
                                        .is_err()
                                    {
                                        warn!("Model input channel closed; cannot forward user message");
                                        break;
                                    }
                                }
                            }
                            Ok(other) => {
                                warn!("Unexpected ServerMessage during Phase 3: {other:?}");
                            }
                            Err(err) => {
                                warn!("Failed to parse server message: {err}");
                            }
                        }
                    }
                    Some(Ok(tungstenite::Message::Ping(data))) => {
                        let _ = ws_sender.send(tungstenite::Message::Pong(data)).await;
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        info!("WebSocket closed by server");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(error = %err, "WebSocket error");
                        break;
                    }
                }
            }

            _ = &mut idle_deadline => {
                info!("Idle timeout reached ({idle_timeout:?}), suspending session");
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
                info!("SIGTERM received, suspending session");
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

    drop(input_tx);
    let _ = ws_sender.send(tungstenite::Message::Close(None)).await;
    Ok(())
}

async fn send_worker_event<Si>(ws_sender: &mut Si, event: SessionEvent) -> bool
where
    Si: Sink<tungstenite::Message> + Unpin,
{
    let msg = WorkerMessage::Event { event };
    let json = match serde_json::to_string(&msg) {
        Ok(j) => j,
        Err(err) => {
            error!(error = %err, "failed to serialize WorkerMessage::Event");
            return false;
        }
    };
    if ws_sender
        .send(tungstenite::Message::Text(json))
        .await
        .is_err()
    {
        warn!("WebSocket closed while sending event");
        return false;
    }
    true
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
    let suspending_event = SessionEvent::Suspending {
        reason: reason.to_string(),
        timestamp: Utc::now(),
    };
    if !send_worker_event(ws_sender, suspending_event).await {
        return;
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

async fn build_session_state_payload(
    home_dir: &Path,
    working_dir: &Path,
    session_id: &str,
) -> SessionStatePayload {
    let path = transcript_path(home_dir, working_dir, session_id);
    let transcript = match tokio::fs::read(&path).await {
        Ok(bytes) => Some(bytes),
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

async fn send_session_state_upload<Si>(
    ws_sender: &mut Si,
    payload: &SessionStatePayload,
) -> Result<()>
where
    Si: Sink<tungstenite::Message> + Unpin,
{
    let data =
        serde_json::to_vec(payload).context("failed to serialize SessionStatePayload to bytes")?;
    let msg = WorkerMessage::SessionStateUpload { data };
    let json = serde_json::to_string(&msg).context("failed to serialize SessionStateUpload")?;
    ws_sender
        .send(tungstenite::Message::Text(json))
        .await
        .map_err(|_| anyhow!("WebSocket send of SessionStateUpload failed"))
}

#[cfg(unix)]
async fn await_sigterm(sig: &mut tokio::signal::unix::Signal) {
    sig.recv().await;
}

#[cfg(not(unix))]
async fn await_sigterm() {
    futures::future::pending::<()>().await;
}
