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
    /// Join handle for the pump task. The pump ends when either the
    /// WebSocket closes or `output_tx` is dropped.
    pub pump: tokio::task::JoinHandle<()>,
}

/// Spawn the Phase-3 pump on a `WorkerSocket` that has already completed
/// Phase 1 + Phase 2. Returns immediately; the caller forwards
/// `input_rx` / `output_tx` to the wrapper's `run_interactive` and awaits
/// `pump` on completion.
pub fn spawn_relay_pump<S>(ws: WorkerSocket<S>) -> RelayAdapter
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
        run_pump(ws, input_tx, output_rx).await;
    });

    RelayAdapter {
        input_rx,
        output_tx,
        pump,
    }
}

async fn run_pump<S>(
    mut ws: WorkerSocket<S>,
    input_tx: mpsc::Sender<WorkerInputMessage>,
    mut output_rx: mpsc::Receiver<WorkerEvent>,
) where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    loop {
        tokio::select! {
            recv = ws.recv() => {
                match recv {
                    Ok(Some(ServerMessage::Event { event })) => {
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
                    Ok(Some(ServerMessage::FirstMessage { agent_prompt, user_message })) => {
                        // Mid-stream FirstMessage: fold into a single UserMessage
                        // so the interactive loop keeps going.
                        let combined = match (agent_prompt.as_str(), user_message.as_str()) {
                            ("", "") => String::new(),
                            ("", u) => u.to_string(),
                            (p, "") => p.to_string(),
                            (p, u) => format!("{p}\n\n{u}"),
                        };
                        if input_tx
                            .send(WorkerInputMessage { content: combined })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Some(other)) => {
                        warn!(?other, "relay pump ignoring unexpected ServerMessage in Phase 3");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        warn!(error = %err, "relay pump WS recv error");
                        break;
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
        let adapter = spawn_relay_pump(ws);
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
}
