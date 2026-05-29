//! Phase-3 bidirectional pump for the interactive worker lifecycle.
//!
//! Per `designs/sessions-worker-run-interface.md` §3.2, after `worker_run`
//! drives Phase 1 (context negotiation) and Phase 2 (first message) on the
//! `WorkerSocket`, it hands the socket to this pump and the per-wrapper
//! input/output channels do the rest:
//!
//! * Inbound `ServerMessage::Event { SessionEvent::UserMessage }` is
//!   translated into a `WorkerInputMessage` and pushed onto `input_tx`.
//! * Outbound `WorkerEvent`s from the wrapper become
//!   `WorkerMessage::Event { SessionEvent::* }` on the WS.
//!
//! If the WS drops mid-run while the model is still producing output (i.e.
//! `output_tx` has not been dropped), the pump invokes the caller-supplied
//! reconnector to reopen the socket and replays the `Reconnecting` →
//! `CatchUp` handshake from §1.6 of the design. `UserMessage`s in the catch-
//! up are re-injected into `input_rx`; all other variants are discarded (the
//! model already emitted them). The model never restarts.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::{Sink, Stream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;
use tracing::warn;

use hydra_common::api::v1::{
    conversations::{ServerMessage, WorkerMessage},
    sessions::SessionEvent,
};

use crate::worker::report::{WorkerEvent, WorkerInputMessage};
use crate::worker::socket::WorkerSocket;

/// Handles returned by [`spawn_relay_pump`].
pub struct RelayAdapter {
    /// Caller-side input receiver. Carries `WorkerInputMessage`s produced from
    /// inbound `ServerMessage::Event { SessionEvent::UserMessage }`. The
    /// caller passes this to `ModelSelector::run_interactive`.
    pub input_rx: mpsc::Receiver<WorkerInputMessage>,
    /// Caller-side output sender. The caller passes this to
    /// `ModelSelector::run_interactive`; the model emits `WorkerEvent`s on it
    /// which the pump forwards onto the WebSocket.
    pub output_tx: mpsc::Sender<WorkerEvent>,
    /// Join handle for the pump task. The pump ends when either the model
    /// channel (`output_tx`) is dropped or a WS reconnect attempt fails.
    pub pump: tokio::task::JoinHandle<()>,
}

/// Async reconnector: invoked by the pump when the WS drops mid-run. Returns
/// a freshly-opened `WorkerSocket` ready for the `Reconnecting` handshake, or
/// an error if reconnection failed (the pump then gives up and exits).
pub type Reconnector<S> =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<WorkerSocket<S>>> + Send>> + Send + Sync>;

/// Spawn the Phase-3 pump on a `WorkerSocket` that has already completed
/// Phase 1 + Phase 2. Returns immediately; the caller forwards
/// `input_rx` / `output_tx` to the wrapper's `run_interactive` and awaits
/// `pump` on completion.
///
/// `initial_session_event_seq` is the number of `session_events` entries the
/// worker is already aware of before Phase 3 starts (i.e. the index of the
/// next event the worker expects to observe). Counted contributions per the
/// `last_received_session_event_index` semantics on the wire:
///
/// * +1 if Phase 1 emitted `SessionEvent::Resumed` (worker → server Event
///   that the server appended to the log).
/// * +1 if Phase 2's `FirstMessage.user_message` was non-empty (the relay
///   dual-wrote that `UserMessage` to the log before folding it).
///
/// The pump increments this counter on every Phase-3 `Event` it sends or
/// receives; on a mid-run WS drop it sends
/// `WorkerMessage::Reconnecting { last_received_session_event_index: seq - 1 }`
/// so the server returns only events strictly past the counter.
pub fn spawn_relay_pump<S>(
    ws: WorkerSocket<S>,
    reconnector: Reconnector<S>,
    initial_session_event_seq: usize,
) -> RelayAdapter
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin
        + Send
        + 'static,
{
    let (input_tx, input_rx) = mpsc::channel::<WorkerInputMessage>(32);
    let (output_tx, output_rx) = mpsc::channel::<WorkerEvent>(32);

    let pump = tokio::spawn(async move {
        run_pump(
            ws,
            input_tx,
            output_rx,
            reconnector,
            initial_session_event_seq,
        )
        .await;
    });

    RelayAdapter {
        input_rx,
        output_tx,
        pump,
    }
}

/// Outcome of the inner pump loop. Distinguishes "model done — exit cleanly"
/// from "WS dropped — attempt reconnect".
enum PumpStep {
    /// Model dropped `output_tx`, or the model-input side closed; finish.
    ModelDone,
    /// WS recv/send failed or closed; attempt reconnect against the
    /// caller's `reconnector`.
    WsDropped,
}

async fn run_pump<S>(
    initial_ws: WorkerSocket<S>,
    input_tx: mpsc::Sender<WorkerInputMessage>,
    mut output_rx: mpsc::Receiver<WorkerEvent>,
    reconnector: Reconnector<S>,
    initial_session_event_seq: usize,
) where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin
        + Send
        + 'static,
{
    let mut ws = initial_ws;
    let mut session_event_seq = initial_session_event_seq;

    loop {
        match pump_one_socket(&mut ws, &input_tx, &mut output_rx, &mut session_event_seq).await {
            PumpStep::ModelDone => return,
            PumpStep::WsDropped => {
                let last_idx = session_event_seq.saturating_sub(1);
                warn!(
                    seq = session_event_seq,
                    last_idx, "WS dropped mid-run; reconnecting"
                );
                let mut new_ws = match (reconnector)().await {
                    Ok(ws) => ws,
                    Err(err) => {
                        warn!(error = %err, "WS reconnect failed; pump exiting");
                        return;
                    }
                };
                if let Err(err) = perform_reconnect_handshake(
                    &mut new_ws,
                    last_idx,
                    &input_tx,
                    &mut session_event_seq,
                )
                .await
                {
                    warn!(error = %err, "Reconnect handshake failed; pump exiting");
                    return;
                }
                ws = new_ws;
            }
        }
    }
}

/// Drive one WS socket from where Phase 3 begins until it either drops or
/// the model side hangs up. Counts Phase-3 events in `session_event_seq`.
async fn pump_one_socket<S>(
    ws: &mut WorkerSocket<S>,
    input_tx: &mpsc::Sender<WorkerInputMessage>,
    output_rx: &mut mpsc::Receiver<WorkerEvent>,
    session_event_seq: &mut usize,
) -> PumpStep
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    loop {
        tokio::select! {
            recv = ws.recv() => {
                match recv {
                    Ok(Some(ServerMessage::Event { event })) => {
                        *session_event_seq += 1;
                        if let SessionEvent::UserMessage { content, .. } = event {
                            if input_tx
                                .send(WorkerInputMessage { content })
                                .await
                                .is_err()
                            {
                                // Model side dropped input_rx — done.
                                return PumpStep::ModelDone;
                            }
                        }
                    }
                    Ok(Some(ServerMessage::FirstMessage { .. })) => {
                        // Per design §1.5, `FirstMessage` is single-shot:
                        // Phase 2 delivers it exactly once and Phase 3
                        // should never see it again. Treat a duplicate as
                        // a server-side ordering bug and drop it rather
                        // than silently feeding it as an extra user turn.
                        warn!(
                            "relay pump dropping stray FirstMessage in Phase 3 \
                             (server-side ordering bug — should be single-shot in Phase 2)"
                        );
                    }
                    Ok(Some(other)) => {
                        warn!(?other, "relay pump ignoring unexpected ServerMessage in Phase 3");
                    }
                    Ok(None) => return PumpStep::WsDropped,
                    Err(err) => {
                        warn!(error = %err, "relay pump WS recv error");
                        return PumpStep::WsDropped;
                    }
                }
            }
            event = output_rx.recv() => {
                match event {
                    Some(event) => {
                        if let Some(api_event) = worker_event_to_session_event(event) {
                            if ws.send(WorkerMessage::Event { event: api_event }).await.is_err() {
                                return PumpStep::WsDropped;
                            }
                            *session_event_seq += 1;
                        }
                    }
                    None => return PumpStep::ModelDone,
                }
            }
        }
    }
}

/// Run the `Reconnecting` → `CatchUp` handshake (§1.6) on a freshly-opened
/// socket. Re-injects every `UserMessage` in the catch-up into `input_tx`
/// and discards every other variant; the model already emitted those. Each
/// event in the catch-up slice advances `session_event_seq`.
async fn perform_reconnect_handshake<S>(
    ws: &mut WorkerSocket<S>,
    last_idx: usize,
    input_tx: &mpsc::Sender<WorkerInputMessage>,
    session_event_seq: &mut usize,
) -> Result<()>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    ws.send(WorkerMessage::Reconnecting {
        last_received_session_event_index: last_idx,
    })
    .await?;
    let reply = ws
        .recv()
        .await?
        .ok_or_else(|| anyhow!("ws closed before CatchUp"))?;
    let events = match reply {
        ServerMessage::CatchUp { events } => events,
        other => {
            return Err(anyhow!(
                "expected CatchUp after Reconnecting, got {other:?}"
            ));
        }
    };
    for event in events {
        *session_event_seq += 1;
        if let SessionEvent::UserMessage { content, .. } = event {
            input_tx
                .send(WorkerInputMessage { content })
                .await
                .map_err(|_| anyhow!("input_rx dropped during catch-up"))?;
        }
    }
    Ok(())
}

fn worker_event_to_session_event(event: WorkerEvent) -> Option<SessionEvent> {
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
    use futures::{SinkExt, StreamExt};
    use std::sync::Mutex;

    /// A minimal sink+stream duplex over futures channels for unit tests.
    type WsFrame = std::result::Result<tungstenite::Message, tungstenite::Error>;
    type WsSender = futures::channel::mpsc::UnboundedSender<WsFrame>;
    type WsReceiver = futures::channel::mpsc::UnboundedReceiver<WsFrame>;

    fn duplex() -> (WorkerSocket<TestStream>, WsSender, WsReceiver) {
        let (server_tx, worker_rx) = futures::channel::mpsc::unbounded::<
            std::result::Result<tungstenite::Message, tungstenite::Error>,
        >();
        let (worker_tx, server_rx) = futures::channel::mpsc::unbounded::<
            std::result::Result<tungstenite::Message, tungstenite::Error>,
        >();
        let ws = WorkerSocket::new(TestStream {
            rx: worker_rx,
            tx: worker_tx,
        });
        (ws, server_tx, server_rx)
    }

    struct TestStream {
        rx: futures::channel::mpsc::UnboundedReceiver<
            std::result::Result<tungstenite::Message, tungstenite::Error>,
        >,
        tx: futures::channel::mpsc::UnboundedSender<
            std::result::Result<tungstenite::Message, tungstenite::Error>,
        >,
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

    /// Reconnector that always fails — tests that never expect reconnect can
    /// install this and assert the pump exits cleanly when the model drops.
    fn failing_reconnector() -> Reconnector<TestStream> {
        Arc::new(|| Box::pin(async { Err(anyhow!("test: no reconnect")) }))
    }

    #[tokio::test]
    async fn pump_forwards_user_messages_to_input_channel() {
        let (ws, mut server_tx, _server_rx) = duplex();
        let adapter = spawn_relay_pump(ws, failing_reconnector(), 0);
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
        };
        let json = serde_json::to_string(&event).unwrap();
        server_tx
            .send(Ok(tungstenite::Message::Text(json)))
            .await
            .unwrap();

        let got = input_rx.recv().await.unwrap();
        assert_eq!(got.content, "hi");

        drop(output_tx);
        drop(server_tx);
        let _ = pump.await;
    }

    /// When the WS drops mid-run, the pump invokes the reconnector, sends
    /// `Reconnecting { last_received_session_event_index }` with the
    /// accumulated seq − 1, reads the `CatchUp` reply, re-injects every
    /// `UserMessage` into `input_rx`, and continues pumping on the new socket.
    #[tokio::test]
    async fn pump_reconnects_and_replays_user_messages_on_ws_drop() {
        // First duplex — the pump consumes one Event, then we drop it to
        // simulate a WS close. The pump should then hit the reconnector.
        let (ws1, mut server_tx1, _server_rx1) = duplex();

        // Second duplex — produced by the reconnector. After Reconnecting +
        // CatchUp the pump should be pumping on this one.
        let (ws2, mut server_tx2, mut server_rx2) = duplex();
        let ws2_slot: Arc<Mutex<Option<WorkerSocket<TestStream>>>> =
            Arc::new(Mutex::new(Some(ws2)));
        let ws2_slot_clone = Arc::clone(&ws2_slot);
        let reconnector: Reconnector<TestStream> = Arc::new(move || {
            let slot = Arc::clone(&ws2_slot_clone);
            Box::pin(async move {
                let mut guard = slot.lock().unwrap();
                guard
                    .take()
                    .ok_or_else(|| anyhow!("reconnector exhausted in test"))
            })
        });

        let RelayAdapter {
            mut input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws1, reconnector, 1);

        // Deliver one UserMessage on the original WS — bumps seq to 2.
        let event = ServerMessage::Event {
            event: SessionEvent::UserMessage {
                content: "pre-drop".to_string(),
                timestamp: chrono::Utc::now(),
            },
        };
        server_tx1
            .send(Ok(tungstenite::Message::Text(
                serde_json::to_string(&event).unwrap(),
            )))
            .await
            .unwrap();
        assert_eq!(input_rx.recv().await.unwrap().content, "pre-drop");

        // Drop the original WS — pump observes Ok(None) and reconnects.
        drop(server_tx1);

        // The pump should send Reconnecting { last_received_session_event_index: 1 }
        // (seq was 2 after pre-drop; saturating_sub(1) = 1).
        let reconnect_frame = server_rx2
            .next()
            .await
            .expect("Reconnecting frame on the new socket")
            .unwrap();
        let tungstenite::Message::Text(text) = reconnect_frame else {
            panic!("expected text frame, got {reconnect_frame:?}");
        };
        let parsed: WorkerMessage = serde_json::from_str(&text).unwrap();
        match parsed {
            WorkerMessage::Reconnecting {
                last_received_session_event_index,
            } => {
                assert_eq!(last_received_session_event_index, 1);
            }
            other => panic!("expected Reconnecting, got {other:?}"),
        }

        // Reply with CatchUp containing a UserMessage (must re-inject), an
        // AssistantMessage (must be discarded — model already emitted it),
        // and another UserMessage (must re-inject in order).
        let catch_up = ServerMessage::CatchUp {
            events: vec![
                SessionEvent::UserMessage {
                    content: "post-drop-1".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                SessionEvent::AssistantMessage {
                    content: "stale-assistant".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                SessionEvent::UserMessage {
                    content: "post-drop-2".to_string(),
                    timestamp: chrono::Utc::now(),
                },
            ],
        };
        server_tx2
            .send(Ok(tungstenite::Message::Text(
                serde_json::to_string(&catch_up).unwrap(),
            )))
            .await
            .unwrap();

        assert_eq!(input_rx.recv().await.unwrap().content, "post-drop-1");
        assert_eq!(input_rx.recv().await.unwrap().content, "post-drop-2");

        // The pump is now driving the new socket. Push another live
        // UserMessage to confirm it survived the reconnect.
        let live = ServerMessage::Event {
            event: SessionEvent::UserMessage {
                content: "after-catch-up".to_string(),
                timestamp: chrono::Utc::now(),
            },
        };
        server_tx2
            .send(Ok(tungstenite::Message::Text(
                serde_json::to_string(&live).unwrap(),
            )))
            .await
            .unwrap();
        assert_eq!(input_rx.recv().await.unwrap().content, "after-catch-up");

        drop(output_tx);
        drop(server_tx2);
        let _ = pump.await;
    }

    /// If the reconnector fails, the pump gives up and exits without panicking
    /// — there is no infinite-retry loop in this PR (per the issue's
    /// out-of-scope note: backoff/retry policy is out of scope).
    #[tokio::test]
    async fn pump_exits_when_reconnect_fails() {
        let (ws, server_tx, _server_rx) = duplex();
        let RelayAdapter {
            input_rx: _input_rx,
            output_tx: _output_tx,
            pump,
        } = spawn_relay_pump(ws, failing_reconnector(), 0);

        drop(server_tx);
        // The pump observes the WS close, calls the failing reconnector,
        // and exits.
        pump.await.expect("pump joins cleanly on failed reconnect");
    }
}
