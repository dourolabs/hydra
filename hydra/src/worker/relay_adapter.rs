//! Phase-3 bidirectional pump for the interactive worker lifecycle.
//!
//! Per `designs/sessions-worker-run-interface.md` §3.2, after
//! `ModelSelector::drive_interactive` runs Phase 1 (context negotiation) and
//! Phase 2 (first message) on the `WorkerSocket`, it hands the socket to this
//! pump and the per-wrapper input/output channels do the rest:
//!
//! * Inbound `ServerMessage::Event { SessionEvent::UserMessage }` is
//!   translated into a `WorkerInputMessage` and pushed onto `input_tx`.
//! * Outbound `WorkerEvent`s from the wrapper become
//!   `WorkerMessage::Event { SessionEvent::* }` on the WS.
//!
//! Mid-session reconnect (design §1.6, §2.2): when `ws.recv()` returns a
//! clean close or transport error while the per-wrapper `output_tx` is
//! still open (i.e. the model is still running), the pump reopens the WS
//! via the supplied [`ReconnectFn`], sends
//! `WorkerMessage::Reconnecting { last_received_session_event_index }`,
//! drains the `ServerMessage::CatchUp { events }` reply, re-injects
//! post-index `UserMessage`s onto `input_tx`, and resumes Phase 3 on the
//! new socket. The model never restarts.

use futures::{Sink, Stream};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;
use tracing::{info, warn};

use hydra_common::api::v1::{
    conversations::{CatchUpEvent, ServerMessage, WorkerMessage},
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

/// Handles returned by [`spawn_relay_pump`].
pub struct RelayAdapter {
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
    /// WebSocket closes and reconnect is exhausted, or `output_tx` is
    /// dropped.
    pub pump: tokio::task::JoinHandle<()>,
}

/// Spawn the Phase-3 pump on a `WorkerSocket` that has already completed
/// Phase 1 + Phase 2. Returns immediately; the caller forwards
/// `input_rx` / `output_tx` to the wrapper's `run_interactive` and awaits
/// `pump` on completion.
pub fn spawn_relay_pump<S>(
    ws: WorkerSocket<S>,
    session_id: SessionId,
    reconnect: ReconnectFn<S>,
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
        run_pump(ws, session_id, reconnect, input_tx, output_rx).await;
    });

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
) where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    // Per-session running max of `event_index` values observed on
    // `ServerMessage::Event { event_index, .. }`. Becomes `Some(N)` once
    // any forwarded event has been seen; sent verbatim on
    // `WorkerMessage::Reconnecting`.
    let mut last_received_session_event_index: Option<usize> = None;

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
                        if let SessionEvent::UserMessage { content, .. } = event {
                            if input_tx
                                .send(WorkerInputMessage { content })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(Some(ServerMessage::FirstMessage { .. })) => {
                        // Per design §1.5, `FirstMessage` is single-shot:
                        // Phase 2 delivers it exactly once and Phase 3
                        // should never see it again. Per §1.6, Phase 2 is
                        // skipped on `Reconnecting`, so a duplicate here
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
                            break;
                        }
                        match attempt_reconnect(
                            &session_id,
                            &reconnect,
                            last_received_session_event_index,
                            &input_tx,
                        )
                        .await
                        {
                            Some((new_ws, new_last_index)) => {
                                ws = new_ws;
                                last_received_session_event_index = new_last_index;
                            }
                            None => break,
                        }
                    }
                }
            }
            event = output_rx.recv() => {
                match event {
                    Some(event) => {
                        if let Some(api_event) = worker_event_to_session_event(event) {
                            if ws.send(WorkerMessage::Event { event: api_event }).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
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
                        if let SessionEvent::UserMessage { content, .. } = event {
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
                    if let SessionEvent::UserMessage { content, .. } = event {
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

/// Push all `UserMessage`s in `events` onto `input_tx` in order, ignoring
/// every other variant (the model already emitted those before the
/// drop). Returns the highest `event_index` observed (or `None` for an
/// empty slice). Exposed as a free function for unit tests.
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
        if let SessionEvent::UserMessage { content, .. } = event {
            if input_tx.send(WorkerInputMessage { content }).await.is_err() {
                return max_index;
            }
        }
    }
    max_index
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
    use futures::SinkExt;

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
