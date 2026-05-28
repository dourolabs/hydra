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
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{Sink, SinkExt};
use hydra_common::{
    api::v1::{
        conversations::{ServerMessage, SessionStatePayload, WorkerConnect, WorkerMessage},
        sessions::SessionEvent,
    },
    SessionId,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};

use crate::client::{HydraClientInterface, RelayWebSocket};
use crate::worker::claude::transcript_path;
use crate::worker::report::{WorkerEvent, WorkerInputMessage};

/// Closure that re-opens the relay WebSocket when the pump observes a
/// transient transport error. Concrete impls in production code call
/// [`HydraClientInterface::connect_relay_websocket`]; tests pass a stub that
/// hands back a paired in-memory stream.
pub type ReconnectFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<RelayWebSocket>> + Send>> + Send + Sync>;

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
///
/// `reconnect` is invoked once when the pump observes a transient WS error
/// in Phase 3 (recv/send failure). The new WS is followed by a
/// [`WorkerConnect::Reconnecting`] handshake carrying the worker's running
/// count of session events; the server's [`ServerMessage::CatchUp`] reply is
/// drained — any `UserMessage` slice is re-injected into `input_tx` so the
/// model sees the messages it missed during the disconnect — and Phase 3
/// resumes on the new WS. A `None` `reconnect` disables the retry (used by
/// tests that want strict single-WS behavior).
///
/// `initial_session_event_count` seeds the worker's running event-count
/// tracker with the value the server reported in
/// `ServerMessage::FirstMessage.session_event_baseline`. Events that landed
/// in `session_events` before Phase 3 — the implicit FirstMessage user
/// message, a `SessionEvent::Resumed` dual-write, the backfilled headless
/// UserMessage — are counted here so the Reconnecting handshake's
/// `last_received_session_event_index` matches the server's index
/// convention. See design §1.5 / §1.6.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pump(
    ws: RelayWebSocket,
    session_id: &SessionId,
    home_dir: PathBuf,
    working_dir: PathBuf,
    idle_timeout: Duration,
    reconnect: Option<ReconnectFn>,
    initial_session_event_count: usize,
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
            reconnect,
            initial_session_event_count,
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

/// Construct a [`ReconnectFn`] that re-opens the relay WS via the supplied
/// [`HydraClientInterface`] for the given session id.
pub fn client_reconnect_fn(
    client: Arc<dyn HydraClientInterface>,
    session_id: SessionId,
) -> ReconnectFn {
    Arc::new(move || {
        let client = Arc::clone(&client);
        let session_id = session_id.clone();
        Box::pin(async move { client.connect_relay_websocket(&session_id).await })
    })
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
    reconnect: Option<ReconnectFn>,
    initial_session_event_count: usize,
) -> Result<()> {
    use futures::StreamExt;
    let (mut ws_sender, mut ws_receiver) = ws.split();

    // Track the model session id reported via WorkerEvent::SessionInit so the
    // suspend-upload code can find the transcript on disk.
    let mut model_session_id: Option<String> = None;

    // Running count of per-session events the worker has either received from
    // the server or successfully sent. Seeded from the server-supplied
    // `ServerMessage::FirstMessage.session_event_baseline` so this count
    // tracks `session_events.len()` on the server side — including events
    // that landed before Phase 3 (the implicit FirstMessage user message, a
    // `SessionEvent::Resumed` dual-write, a backfilled headless UserMessage).
    // Used to populate
    // `WorkerConnect::Reconnecting.last_received_session_event_index` so the
    // server can ship every event past that point in `CatchUp`.
    let mut session_event_count: usize = initial_session_event_count;

    let idle_deadline = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_deadline);

    #[cfg(unix)]
    let mut sigterm_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;

    // Whether we've already used our one reconnect attempt. The protocol
    // expects at most one mid-session reconnect; further failures terminate
    // the pump cleanly.
    let mut reconnect_used = false;

    'outer: loop {
        tokio::select! {
            event = output_rx.recv() => {
                match event {
                    Some(WorkerEvent::AssistantText { text }) => {
                        if !text.is_empty() {
                            let session_event = SessionEvent::AssistantMessage {
                                content: text,
                                timestamp: Utc::now(),
                            };
                            if send_worker_event(&mut ws_sender, session_event).await {
                                session_event_count = session_event_count.saturating_add(1);
                            } else {
                                if let Some((new_sender, new_receiver)) = try_reconnect(
                                    &reconnect,
                                    &mut reconnect_used,
                                    session_id,
                                    session_event_count,
                                    &input_tx,
                                    &mut session_event_count,
                                ).await {
                                    ws_sender = new_sender;
                                    ws_receiver = new_receiver;
                                    continue 'outer;
                                }
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
                        if send_worker_event(&mut ws_sender, session_event).await {
                            session_event_count = session_event_count.saturating_add(1);
                        } else {
                            if let Some((new_sender, new_receiver)) = try_reconnect(
                                &reconnect,
                                &mut reconnect_used,
                                session_id,
                                session_event_count,
                                &input_tx,
                                &mut session_event_count,
                            ).await {
                                ws_sender = new_sender;
                                ws_receiver = new_receiver;
                                continue 'outer;
                            }
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
                                session_event_count = session_event_count.saturating_add(1);
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
                        if let Some((new_sender, new_receiver)) = try_reconnect(
                            &reconnect,
                            &mut reconnect_used,
                            session_id,
                            session_event_count,
                            &input_tx,
                            &mut session_event_count,
                        ).await {
                            ws_sender = new_sender;
                            ws_receiver = new_receiver;
                            continue 'outer;
                        }
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(error = %err, "WebSocket error");
                        if let Some((new_sender, new_receiver)) = try_reconnect(
                            &reconnect,
                            &mut reconnect_used,
                            session_id,
                            session_event_count,
                            &input_tx,
                            &mut session_event_count,
                        ).await {
                            ws_sender = new_sender;
                            ws_receiver = new_receiver;
                            continue 'outer;
                        }
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

/// Attempt one Phase-3 reconnect via the caller-supplied closure. On success
/// returns the split halves of the new WS; on failure (or if `reconnect` is
/// `None`, or if the slot has already been used) returns `None` so the caller
/// can tear down the pump.
async fn try_reconnect(
    reconnect: &Option<ReconnectFn>,
    reconnect_used: &mut bool,
    session_id: &SessionId,
    current_event_count: usize,
    input_tx: &mpsc::Sender<WorkerInputMessage>,
    out_event_count: &mut usize,
) -> Option<(
    futures::stream::SplitSink<RelayWebSocket, tungstenite::Message>,
    futures::stream::SplitStream<RelayWebSocket>,
)> {
    use futures::StreamExt;
    let reconnect = reconnect.as_ref()?;
    if *reconnect_used {
        warn!(%session_id, "WS reconnect slot already used; exiting pump");
        return None;
    }
    *reconnect_used = true;

    let new_ws = match (reconnect)().await {
        Ok(ws) => ws,
        Err(err) => {
            warn!(%session_id, error = %err, "WS reconnect failed");
            return None;
        }
    };
    let (mut new_sender, mut new_receiver) = new_ws.split();

    // Convention: `last_received_session_event_index` is the index of the
    // most recent event the worker has observed. When the worker has seen
    // `N` events, the highest index it has confirmed is `N - 1`; when `N`
    // is 0 (rare — disconnect before any Phase-3 traffic) we send `0` and
    // accept the narrow window in which event-0 might be skipped.
    let last_idx = current_event_count.saturating_sub(1);
    let msg = WorkerMessage::Connect(WorkerConnect::Reconnecting {
        last_received_session_event_index: last_idx,
    });
    let json = match serde_json::to_string(&msg) {
        Ok(j) => j,
        Err(err) => {
            error!(%session_id, error = %err, "failed to serialize Reconnecting handshake");
            return None;
        }
    };
    if new_sender
        .send(tungstenite::Message::Text(json))
        .await
        .is_err()
    {
        warn!(%session_id, "failed to send Reconnecting handshake on new WS");
        return None;
    }
    info!(%session_id, last_received_session_event_index = last_idx, "sent Reconnecting handshake");

    // Wait for CatchUp, skipping pings.
    loop {
        let frame = match new_receiver.next().await {
            Some(Ok(f)) => f,
            Some(Err(err)) => {
                warn!(%session_id, error = %err, "WS error awaiting CatchUp");
                return None;
            }
            None => {
                warn!(%session_id, "WS closed awaiting CatchUp");
                return None;
            }
        };
        match frame {
            tungstenite::Message::Text(text) => {
                match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(ServerMessage::CatchUp { events }) => {
                        info!(%session_id, events = events.len(), "CatchUp received");
                        for event in events {
                            *out_event_count = out_event_count.saturating_add(1);
                            // Per design §1.6, only UserMessages get re-injected
                            // into the model's input queue; the rest are
                            // discarded (we already emitted those as the prior
                            // pump run).
                            if let SessionEvent::UserMessage { content, .. } = event {
                                if input_tx.send(WorkerInputMessage { content }).await.is_err() {
                                    warn!(%session_id, "input channel closed during CatchUp drain");
                                    return None;
                                }
                            }
                        }
                        return Some((new_sender, new_receiver));
                    }
                    Ok(other) => {
                        warn!(%session_id, ?other, "expected CatchUp, got other ServerMessage; exiting");
                        return None;
                    }
                    Err(err) => {
                        warn!(%session_id, error = %err, "failed to parse CatchUp");
                        return None;
                    }
                }
            }
            tungstenite::Message::Ping(data) => {
                let _ = new_sender.send(tungstenite::Message::Pong(data)).await;
            }
            tungstenite::Message::Close(_) => {
                warn!(%session_id, "WS closed before CatchUp arrived");
                return None;
            }
            _ => continue,
        }
    }
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
///
/// Note on `session_event_count`: this helper does NOT increment the pump's
/// running event count even though the Suspending event lands in
/// `session_events`. The pump terminates immediately after this call (no
/// further Phase-3 work can happen), and the WS is closed via
/// `Message::Close` right after — so the worker will never need to emit a
/// Reconnecting handshake whose `last_received_session_event_index` would
/// depend on the post-Suspending count. If that lifecycle ever changes
/// (e.g., a future Suspending → resumed flow that keeps the pump alive),
/// add a `session_event_count = session_event_count.saturating_add(1)` at
/// the caller per the design's index-tracking convention.
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
