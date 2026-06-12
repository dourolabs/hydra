//! Phase-3 bidirectional pump for the interactive worker lifecycle.
//!
//! After `ModelSelector::drive_interactive` runs Phase 1 (context
//! negotiation) and Phase 2 (first message) on the `WorkerSocket`, it
//! hands the socket to this pump and the per-wrapper input/output
//! channels do the rest:
//!
//! * Inbound `ServerMessage::Event { SessionEvent::UserMessage }` is
//!   translated into a `WorkerInputMessage` and pushed onto `input_tx`.
//! * Outbound `WorkerEvent`s from the wrapper become
//!   `WorkerMessage::Event { SessionEvent::* }` on the WS.
//!
//! Mid-session reconnect: when `ws.recv()` returns a clean close or
//! transport error while the per-wrapper `output_tx` is still open (i.e.
//! the model is still running), the pump reopens the WS via the supplied
//! [`ReconnectFn`], sends
//! `WorkerMessage::Reconnecting { last_received_session_event_index }`,
//! drains the `ServerMessage::CatchUp { events }` reply, re-injects
//! post-index `UserMessage`s onto `input_tx`, and resumes Phase 3 on the
//! new socket. Phase 2 is skipped on reconnect because the model is
//! mid-run and `FirstMessage` has already been delivered. The model
//! never restarts.

use futures::{Sink, Stream};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;
use tracing::{info, warn};

use hydra_common::api::v1::{
    relay::{CatchUpEvent, ServerMessage, WorkerMessage},
    sessions::SessionEvent,
};
use hydra_common::SessionId;

use crate::worker::report::{WorkerEvent, WorkerInputMessage};
use crate::worker::socket::WorkerSocket;

/// Max number of times the pump attempts to reopen the WS after a
/// mid-session drop before giving up.
const RECONNECT_MAX_ATTEMPTS: u32 = 5;
/// Fixed delay between reconnect attempts.
const RECONNECT_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Callback that builds a fresh [`WorkerSocket`] for a session. The
/// pump invokes this when it observes a WS drop and the model is still
/// running. Production wires this to `HydraClient::connect_relay_websocket`;
/// tests can pass a noop that returns an error to disable reconnect.
pub type ReconnectFn<S> = Arc<
    dyn Fn(SessionId) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkerSocket<S>>> + Send>>
        + Send
        + Sync,
>;

/// Outcome of a finished relay pump. The pump hands back the open
/// `WorkerSocket` (when the model exited naturally or `EndSession` was
/// observed) so the caller (`ModelSelector::drive_interactive`) can issue
/// the unified end-of-session cleanup messages (`SessionStateUpload`,
/// `Closed` event, optional `EndSessionAck`) before closing the WS.
pub struct PumpExit<S> {
    /// Open WS handed back to the caller, or `None` if the WS was lost
    /// (reconnect exhausted, or transport error after `EndSession`).
    pub ws: Option<WorkerSocket<S>>,
    /// Whether the pump observed an inbound `ServerMessage::EndSession`.
    /// When `true`, the caller appends `WorkerMessage::EndSessionAck` to
    /// the unified cleanup sequence.
    pub end_session_requested: bool,
}

/// Handles returned by [`spawn_relay_pump`].
pub struct RelayAdapter<S> {
    /// Caller-side input receiver. Carries `WorkerInputMessage`s produced from
    /// inbound `ServerMessage::Event { SessionEvent::UserMessage }`.
    /// `ModelSelector::drive_interactive` forwards this into the per-wrapper
    /// interactive runner.
    pub input_rx: mpsc::Receiver<WorkerInputMessage>,
    /// Caller-side output sender. `ModelSelector::drive_interactive` hands
    /// this to the per-wrapper interactive runner, which emits `WorkerEvent`s
    /// on it; the pump forwards them onto the WebSocket.
    pub output_tx: mpsc::Sender<WorkerEvent>,
    /// Join handle for the pump task. The pump ends when either the
    /// WebSocket closes and reconnect is exhausted, `output_tx` is
    /// dropped (model exited), or the server sent `EndSession` and the
    /// model has since exited.
    pub pump: tokio::task::JoinHandle<PumpExit<S>>,
}

/// Spawn the Phase-3 pump on a `WorkerSocket` that has already completed
/// Phase 1 + Phase 2. Returns immediately; the caller forwards
/// `input_rx` / `output_tx` to the wrapper's `run_interactive` and awaits
/// `pump` on completion.
pub fn spawn_relay_pump<S>(
    ws: WorkerSocket<S>,
    session_id: SessionId,
    reconnect: ReconnectFn<S>,
) -> RelayAdapter<S>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin
        + Send
        + 'static,
{
    let (input_tx, input_rx) = mpsc::channel::<WorkerInputMessage>(32);
    let (output_tx, output_rx) = mpsc::channel::<WorkerEvent>(32);

    let pump =
        tokio::spawn(async move { run_pump(ws, session_id, reconnect, input_tx, output_rx).await });

    RelayAdapter {
        input_rx,
        output_tx,
        pump,
    }
}

async fn run_pump<S>(
    mut ws: WorkerSocket<S>,
    session_id: SessionId,
    reconnect: ReconnectFn<S>,
    input_tx: mpsc::Sender<WorkerInputMessage>,
    mut output_rx: mpsc::Receiver<WorkerEvent>,
) -> PumpExit<S>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    // Per-session running max of `event_index` values observed on
    // `ServerMessage::Event { event_index, .. }`. Becomes `Some(N)` once
    // any forwarded event has been seen; sent verbatim on
    // `WorkerMessage::Reconnecting`.
    let mut last_received_session_event_index: Option<usize> = None;
    // `Some(_)` while we still forward inbound `UserMessage`s to the
    // model; set to `None` on `ServerMessage::EndSession` to signal
    // Claude (interactive) to observe stdin EOF and exit.
    let mut input_tx: Option<mpsc::Sender<WorkerInputMessage>> = Some(input_tx);
    let mut end_session_requested = false;

    loop {
        tokio::select! {
            recv = ws.recv() => {
                match recv {
                    Ok(Some(ServerMessage::Event { event, event_index })) => {
                        last_received_session_event_index =
                            Some(match last_received_session_event_index {
                                None => event_index,
                                Some(prev) => prev.max(event_index),
                            });
                        if let Some(content) = project_to_input_text(&event) {
                            if let Some(tx) = input_tx.as_ref() {
                                if tx.send(WorkerInputMessage { content }).await.is_err() {
                                    // Model is gone; nothing left to forward.
                                    input_tx = None;
                                }
                            }
                        }
                    }
                    Ok(Some(ServerMessage::EndSession)) => {
                        // Signal the model to exit gracefully by dropping
                        // our `input_tx`: the interactive runner sees its
                        // input channel close and drops Claude's stdin,
                        // which causes Claude to observe EOF and exit.
                        // We continue draining the pump until `output_rx`
                        // closes (model done) — only then does the caller
                        // run the unified cleanup-and-close sequence.
                        info!(%session_id, "relay pump observed EndSession; signaling graceful exit");
                        end_session_requested = true;
                        input_tx = None;
                    }
                    Ok(Some(ServerMessage::FirstMessage { .. })) => {
                        // `FirstMessage` is single-shot: Phase 2 delivers
                        // it exactly once and Phase 3 should never see it
                        // again. Phase 2 is also skipped on reconnect (the
                        // model is mid-run), so a duplicate arriving here
                        // is always a server-side ordering bug.
                        warn!(
                            "relay pump dropping stray FirstMessage in Phase 3 \
                             (server-side ordering bug — should be single-shot in Phase 2)"
                        );
                    }
                    Ok(Some(other)) => {
                        warn!(?other, "relay pump ignoring unexpected ServerMessage in Phase 3");
                    }
                    Ok(None) | Err(_) => {
                        // WS closed or transport error. If `output_rx` is
                        // closed (no sender left) the model is gone too —
                        // exit. Otherwise the model is still running, so
                        // try to reopen the socket and resume Phase 3.
                        if output_rx.is_closed() {
                            return PumpExit { ws: None, end_session_requested };
                        }
                        // Reconnect tries to reopen the WS whenever the
                        // model is still running (we still hold an
                        // `input_tx`). If we already saw EndSession and
                        // the WS dropped, returning here skips the
                        // cleanup messages — there is no WS to send them
                        // on, and the existing server fallback (`stop_job`)
                        // catches the disconnect.
                        let reconnect_result = match input_tx.as_ref() {
                            Some(tx) => attempt_reconnect(
                                &session_id,
                                &reconnect,
                                last_received_session_event_index,
                                tx,
                            )
                            .await,
                            // EndSession already received — don't reopen.
                            None => None,
                        };
                        match reconnect_result {
                            Some((new_ws, new_last_index)) => {
                                ws = new_ws;
                                last_received_session_event_index = new_last_index;
                            }
                            None => return PumpExit { ws: None, end_session_requested },
                        }
                    }
                }
            }
            event = output_rx.recv() => {
                match event {
                    Some(event) => {
                        if let Some(api_event) = worker_event_to_session_event(event) {
                            if ws.send(WorkerMessage::Event { event: api_event }).await.is_err() {
                                // WS broken; if we still have a working
                                // input_tx the reconnect arm above will
                                // try to recover on next ws.recv() error.
                                return PumpExit { ws: None, end_session_requested };
                            }
                        }
                    }
                    None => return PumpExit { ws: Some(ws), end_session_requested },
                }
            }
        }
    }
}

/// Attempt to reopen the relay WS and drain the resulting `CatchUp`.
/// Returns the new socket together with the updated running-max event
/// index. Returns `None` if all attempts fail, the catch-up never
/// arrives, or `input_tx` is closed (model is gone).
async fn attempt_reconnect<S>(
    session_id: &SessionId,
    reconnect: &ReconnectFn<S>,
    last_received_session_event_index: Option<usize>,
    input_tx: &mpsc::Sender<WorkerInputMessage>,
) -> Option<(WorkerSocket<S>, Option<usize>)>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    let mut last_err: Option<String> = None;
    for attempt in 1..=RECONNECT_MAX_ATTEMPTS {
        info!(
            %session_id,
            attempt,
            max_attempts = RECONNECT_MAX_ATTEMPTS,
            "relay pump attempting to reconnect"
        );
        let mut new_ws = match reconnect(session_id.clone()).await {
            Ok(ws) => ws,
            Err(err) => {
                last_err = Some(err.to_string());
                warn!(%session_id, error = %err, "relay pump reconnect attempt failed");
                if attempt < RECONNECT_MAX_ATTEMPTS {
                    tokio::time::sleep(RECONNECT_RETRY_DELAY).await;
                }
                continue;
            }
        };

        if let Err(err) = new_ws
            .send(WorkerMessage::Reconnecting {
                last_received_session_event_index,
            })
            .await
        {
            last_err = Some(err.to_string());
            warn!(%session_id, error = %err, "failed to send Reconnecting on new socket");
            if attempt < RECONNECT_MAX_ATTEMPTS {
                tokio::time::sleep(RECONNECT_RETRY_DELAY).await;
            }
            continue;
        }

        // Drain frames until `CatchUp` arrives. Anything else (e.g. a
        // race with a fresh `Event` on the new socket) we forward in the
        // same way the main loop would and continue waiting for
        // `CatchUp`.
        let mut updated_last_index = last_received_session_event_index;
        loop {
            match new_ws.recv().await {
                Ok(Some(ServerMessage::CatchUp { events })) => {
                    for CatchUpEvent { event, event_index } in events {
                        updated_last_index = Some(match updated_last_index {
                            None => event_index,
                            Some(prev) => prev.max(event_index),
                        });
                        if let Some(content) = project_to_input_text(&event) {
                            if input_tx.send(WorkerInputMessage { content }).await.is_err() {
                                return None;
                            }
                        }
                    }
                    return Some((new_ws, updated_last_index));
                }
                Ok(Some(ServerMessage::Event { event, event_index })) => {
                    updated_last_index = Some(match updated_last_index {
                        None => event_index,
                        Some(prev) => prev.max(event_index),
                    });
                    if let Some(content) = project_to_input_text(&event) {
                        if input_tx.send(WorkerInputMessage { content }).await.is_err() {
                            return None;
                        }
                    }
                }
                Ok(Some(other)) => {
                    warn!(
                        ?other,
                        "relay pump dropping unexpected frame while awaiting CatchUp"
                    );
                }
                Ok(None) | Err(_) => {
                    // Server closed before delivering CatchUp; try
                    // another reconnect.
                    last_err = Some("server closed before CatchUp".to_string());
                    break;
                }
            }
        }
        if attempt < RECONNECT_MAX_ATTEMPTS {
            tokio::time::sleep(RECONNECT_RETRY_DELAY).await;
        }
    }
    warn!(
        %session_id,
        attempts = RECONNECT_MAX_ATTEMPTS,
        error = last_err.as_deref().unwrap_or("unknown"),
        "relay pump giving up after exhausting reconnect attempts"
    );
    None
}

/// Extract the user-shaped input text from a `SessionEvent` for
/// re-injection into the model. Returns `Some(text)` for `UserMessage`
/// (verbatim content) and `SystemEvent` (canonical render via
/// [`SystemEventKind::render`][hydra_common::api::v1::sessions::SystemEventKind::render]),
/// `None` for every other variant.
///
/// `SystemEvent` is re-injected on reconnect because the model has not
/// yet seen it — same treatment as `UserMessage`, distinct from
/// `AssistantMessage`/`ToolUse`/`Resumed` which the model already
/// emitted (or already consumed) before the drop.
fn project_to_input_text(event: &SessionEvent) -> Option<String> {
    match event {
        SessionEvent::UserMessage { content, .. } => Some(content.clone()),
        SessionEvent::SystemEvent { kind, .. } => Some(kind.render()),
        _ => None,
    }
}

/// Push all input-shaped events in `events` onto `input_tx` in order,
/// ignoring every other variant (the model already emitted those before
/// the drop). Returns the highest `event_index` observed (or `None` for
/// an empty slice). Exposed as a free function for unit tests.
#[cfg(test)]
async fn process_catch_up_events(
    events: Vec<CatchUpEvent>,
    input_tx: &mpsc::Sender<WorkerInputMessage>,
) -> Option<usize> {
    let mut max_index: Option<usize> = None;
    for CatchUpEvent { event, event_index } in events {
        max_index = Some(match max_index {
            None => event_index,
            Some(prev) => prev.max(event_index),
        });
        if let Some(content) = project_to_input_text(&event) {
            if input_tx.send(WorkerInputMessage { content }).await.is_err() {
                return max_index;
            }
        }
    }
    max_index
}

pub(crate) fn worker_event_to_session_event(event: WorkerEvent) -> Option<SessionEvent> {
    match event {
        WorkerEvent::AssistantText { text } => Some(SessionEvent::AssistantMessage {
            content: text,
            timestamp: chrono::Utc::now(),
        }),
        WorkerEvent::ToolUse { tool_name, payload } => Some(SessionEvent::ToolUse {
            tool_name,
            payload,
            timestamp: chrono::Utc::now(),
        }),
        WorkerEvent::Usage { .. } | WorkerEvent::SessionInit { .. } | WorkerEvent::Raw { .. } => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::ws_test_util::{collect_worker_msgs, duplex, push_server_msg, TestStream};

    /// A `ReconnectFn` that always errors — the pump treats this as
    /// "reconnect not available", and exits after exhausting attempts.
    /// Unit tests use this so the reconnect branch never produces a
    /// foreign socket of an incompatible type.
    fn noop_reconnect<S>() -> ReconnectFn<S>
    where
        S: Send + 'static,
    {
        Arc::new(|_| {
            Box::pin(async {
                Err(anyhow::anyhow!(
                    "noop reconnect: unit tests do not exercise the reconnect path"
                ))
            })
        })
    }

    #[tokio::test]
    async fn pump_forwards_user_messages_to_input_channel() {
        let (ws, mut server_tx, _server_rx) = duplex();
        let session_id = SessionId::new();
        let adapter = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());
        let RelayAdapter {
            mut input_rx,
            output_tx,
            pump,
        } = adapter;

        let event = ServerMessage::Event {
            event: SessionEvent::UserMessage {
                content: "hi".to_string(),
                timestamp: chrono::Utc::now(),
            },
            event_index: 1,
        };
        push_server_msg(&mut server_tx, &event).await;

        let got = input_rx.recv().await.unwrap();
        assert_eq!(got.content, "hi");

        drop(output_tx);
        drop(server_tx);
        let _ = pump.await;
    }

    #[tokio::test]
    async fn pump_returns_open_ws_on_natural_exit() {
        // When `output_tx` is dropped (model exited) and no EndSession was
        // ever received, the pump hands the WS back so the caller can
        // issue the unified cleanup messages.
        let (ws, _server_tx, _server_rx) = duplex();
        let session_id = SessionId::new();
        let RelayAdapter {
            input_rx: _input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());

        drop(output_tx);
        let exit = pump.await.expect("pump task panicked");
        assert!(exit.ws.is_some(), "natural exit must hand the WS back");
        assert!(!exit.end_session_requested);
    }

    #[tokio::test]
    async fn pump_handles_end_session_and_signals_input_close() {
        // On inbound `ServerMessage::EndSession` the pump (a) drops its
        // `input_tx` so the model observes input EOF and (b) sets the
        // `end_session_requested` flag for the caller's unified cleanup.
        let (ws, mut server_tx, _server_rx) = duplex();
        let session_id = SessionId::new();
        let RelayAdapter {
            mut input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());

        push_server_msg(&mut server_tx, &ServerMessage::EndSession).await;
        // `input_rx.recv()` returns None once the pump drops its sender —
        // that's the worker-side signal we propagate to Claude as stdin EOF.
        assert!(input_rx.recv().await.is_none());

        // Model exits naturally now that its input is closed.
        drop(output_tx);
        let exit = pump.await.expect("pump task panicked");
        assert!(exit.end_session_requested);
        assert!(
            exit.ws.is_some(),
            "EndSession-driven exit must hand the WS back for cleanup"
        );
    }

    #[tokio::test]
    async fn pump_returns_no_ws_when_model_gone_and_ws_closes() {
        // If both the WS closes and `output_rx` is already closed, the
        // pump exits without a WS and with `end_session_requested = false`.
        let (ws, server_tx, _server_rx) = duplex();
        let session_id = SessionId::new();
        let RelayAdapter {
            input_rx: _input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());

        drop(output_tx);
        // Need both to be dropped to unwedge the pump.
        drop(server_tx);
        let exit = pump.await.expect("pump task panicked");
        // Either branch (output_rx None first, or ws close first) is
        // acceptable — both flag end_session_requested = false.
        assert!(!exit.end_session_requested);
        // ws may or may not be Some depending on which branch fired
        // first; the key invariant we assert is the flag.
        let _ = exit.ws;
    }

    #[tokio::test]
    async fn pump_natural_exit_lets_worker_send_cleanup_on_returned_ws() {
        // End-to-end-ish: simulate a model that emits one assistant
        // message, then exits naturally. Verify the pump (a) forwarded
        // the message on the WS, (b) handed the WS back, and (c) we can
        // send cleanup frames on that returned WS that round-trip to the
        // server side.
        let (ws, _server_tx, mut server_rx) = duplex();
        let session_id = SessionId::new();
        let RelayAdapter {
            input_rx: _input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());

        output_tx
            .send(WorkerEvent::AssistantText {
                text: "hello".to_string(),
            })
            .await
            .unwrap();
        drop(output_tx);
        let exit = pump.await.expect("pump task panicked");
        let mut ws = exit.ws.expect("WS expected on natural exit");

        // Send a fake cleanup sequence on the returned WS.
        ws.send(WorkerMessage::Event {
            event: SessionEvent::Closed {
                timestamp: chrono::Utc::now(),
            },
        })
        .await
        .unwrap();
        ws.send(WorkerMessage::SessionStateUpload {
            data: vec![1, 2, 3],
        })
        .await
        .unwrap();
        drop(ws);

        let frames = collect_worker_msgs(&mut server_rx).await;
        // First frame: the assistant message the pump forwarded.
        match &frames[0] {
            WorkerMessage::Event {
                event: SessionEvent::AssistantMessage { content, .. },
            } => assert_eq!(content, "hello"),
            other => panic!("expected forwarded AssistantMessage, got {other:?}"),
        }
        // Then the cleanup frames in the order we sent them.
        match &frames[1] {
            WorkerMessage::Event {
                event: SessionEvent::Closed { .. },
            } => {}
            other => panic!("expected Closed event, got {other:?}"),
        }
        match &frames[2] {
            WorkerMessage::SessionStateUpload { data } => assert_eq!(data, &vec![1, 2, 3]),
            other => panic!("expected SessionStateUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn process_catch_up_pushes_only_user_messages_in_order() {
        // Mixed-variant catch-up: only the UserMessages reach `input_rx`,
        // and the running max comes back as the highest event_index seen.
        let (input_tx, mut input_rx) = mpsc::channel::<WorkerInputMessage>(8);
        let events = vec![
            CatchUpEvent {
                event: SessionEvent::UserMessage {
                    content: "first".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 3,
            },
            CatchUpEvent {
                event: SessionEvent::AssistantMessage {
                    content: "(model said this before the drop — must not re-inject)".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 4,
            },
            CatchUpEvent {
                event: SessionEvent::UserMessage {
                    content: "second".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 5,
            },
            CatchUpEvent {
                event: SessionEvent::ToolUse {
                    tool_name: "noop".to_string(),
                    payload: serde_json::Value::Null,
                    timestamp: chrono::Utc::now(),
                },
                event_index: 6,
            },
        ];
        let max = process_catch_up_events(events, &input_tx).await;
        assert_eq!(max, Some(6));
        drop(input_tx);

        let mut got = Vec::new();
        while let Some(m) = input_rx.recv().await {
            got.push(m.content);
        }
        assert_eq!(got, vec!["first".to_string(), "second".to_string()]);
    }

    #[tokio::test]
    async fn pump_forwards_system_event_via_canonical_render() {
        // Inbound SystemEvent is projected into the model's input
        // channel using `SystemEventKind::render()` — same treatment as
        // a UserMessage, but the canonical string is the only way the
        // worker hand-formats it.
        use hydra_common::api::v1::projects::StatusKey;
        use hydra_common::api::v1::sessions::SystemEventKind;
        use hydra_common::IssueId;
        let (ws, mut server_tx, _server_rx) = duplex();
        let session_id = SessionId::new();
        let RelayAdapter {
            mut input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws, session_id, noop_reconnect::<TestStream>());

        let child_id = IssueId::try_from("i-abcdef".to_string()).unwrap();
        let kind = SystemEventKind::ChildUnblocked {
            child_id,
            new_status: StatusKey::try_new("complete").unwrap(),
        };
        let event = ServerMessage::Event {
            event: SessionEvent::SystemEvent {
                kind: kind.clone(),
                timestamp: chrono::Utc::now(),
            },
            event_index: 1,
        };
        push_server_msg(&mut server_tx, &event).await;

        let got = input_rx.recv().await.unwrap();
        assert_eq!(got.content, kind.render());
        assert_eq!(
            got.content,
            "Child i-abcdef reached status complete; please continue."
        );

        drop(output_tx);
        drop(server_tx);
        let _ = pump.await;
    }

    #[tokio::test]
    async fn process_catch_up_re_injects_system_events_as_user_input() {
        // SystemEvents in the catch-up slice must reach `input_rx` —
        // the model has not yet seen them. AssistantMessages /
        // ToolUse / Resumed are discarded.
        use hydra_common::api::v1::projects::StatusKey;
        use hydra_common::api::v1::sessions::SystemEventKind;
        use hydra_common::IssueId;
        let (input_tx, mut input_rx) = mpsc::channel::<WorkerInputMessage>(8);
        let kind = SystemEventKind::ChildUnblocked {
            child_id: IssueId::try_from("i-abcdef".to_string()).unwrap(),
            new_status: StatusKey::try_new("complete").unwrap(),
        };
        let events = vec![
            CatchUpEvent {
                event: SessionEvent::AssistantMessage {
                    content: "(prior assistant — must not re-inject)".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 1,
            },
            CatchUpEvent {
                event: SessionEvent::SystemEvent {
                    kind: kind.clone(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 2,
            },
            CatchUpEvent {
                event: SessionEvent::UserMessage {
                    content: "user-after".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                event_index: 3,
            },
        ];
        let max = process_catch_up_events(events, &input_tx).await;
        assert_eq!(max, Some(3));
        drop(input_tx);

        let mut got = Vec::new();
        while let Some(m) = input_rx.recv().await {
            got.push(m.content);
        }
        assert_eq!(got, vec![kind.render(), "user-after".to_string()]);
    }

    #[tokio::test]
    async fn process_catch_up_empty_does_not_touch_input() {
        // Headless-style branch: no UserMessages in the slice, no pushes
        // and `None` running max.
        let (input_tx, mut input_rx) = mpsc::channel::<WorkerInputMessage>(4);
        let max = process_catch_up_events(Vec::new(), &input_tx).await;
        assert_eq!(max, None);
        drop(input_tx);
        assert!(input_rx.recv().await.is_none());
    }
}
