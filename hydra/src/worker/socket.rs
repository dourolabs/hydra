//! Typed wrapper around the worker-side WebSocket stream.
//!
//! Per `designs/sessions-worker-run-interface.md` §3.2 — `WorkerSocket` is a
//! narrow newtype that exposes typed `send` / `recv` for [`WorkerMessage`] /
//! [`ServerMessage`] only, so the dispatch layer ([`crate::worker::ModelSelector`])
//! sees the message vocabulary and not the HTTP / tungstenite stack.
//!
//! This module lands as inert library code in PR-1. PR-3 wires it into
//! `worker_run.rs` and `model_selector.rs` and removes the
//! [`crate::worker::relay_adapter`] path.

use anyhow::{anyhow, Context, Result};
use futures::{Sink, SinkExt, Stream, StreamExt};
use hydra_common::api::v1::conversations::{ServerMessage, WorkerMessage};
use tokio_tungstenite::tungstenite;

/// A typed worker-side wrapper around a WebSocket stream.
///
/// `S` is the underlying Sink+Stream of [`tungstenite::Message`]. Tests use a
/// `futures::channel::mpsc`-based duplex; production code uses the connected
/// [`crate::client::RelayWebSocket`]. The wrapper handles JSON
/// serialization, control-frame plumbing (pings auto-Pong, Close ends the
/// stream cleanly), and discards non-text frames.
pub struct WorkerSocket<S> {
    inner: S,
}

impl<S> WorkerSocket<S>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    /// Wrap an existing WebSocket stream.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// Send one [`WorkerMessage`] up to the server.
    pub async fn send(&mut self, msg: WorkerMessage) -> Result<()> {
        let json = serde_json::to_string(&msg).context("failed to serialize WorkerMessage")?;
        self.inner
            .send(tungstenite::Message::Text(json))
            .await
            .map_err(|e| anyhow!("WorkerSocket send failed: {e}"))
    }

    /// Receive the next [`ServerMessage`] from the server.
    ///
    /// Returns `Ok(None)` on a clean WS close. Pings are auto-Ponged and
    /// other non-text frames are skipped silently.
    pub async fn recv(&mut self) -> Result<Option<ServerMessage>> {
        loop {
            let Some(frame) = self.inner.next().await else {
                return Ok(None);
            };
            let frame = frame.map_err(|e| anyhow!("WorkerSocket recv WS error: {e}"))?;
            match frame {
                tungstenite::Message::Text(text) => {
                    let parsed: ServerMessage = serde_json::from_str(&text)
                        .with_context(|| format!("failed to parse ServerMessage: {text}"))?;
                    return Ok(Some(parsed));
                }
                tungstenite::Message::Ping(payload) => {
                    self.inner
                        .send(tungstenite::Message::Pong(payload))
                        .await
                        .map_err(|e| anyhow!("WorkerSocket pong failed: {e}"))?;
                }
                tungstenite::Message::Close(_) => return Ok(None),
                // Binary / Pong / Frame — not used in the worker protocol; skip.
                _ => continue,
            }
        }
    }

    /// Consume the wrapper and return the underlying sink+stream. Useful when
    /// the caller wants to drive a clean WS close itself.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use futures::{channel::mpsc, SinkExt, StreamExt};
    use hydra_common::api::v1::{
        conversations::{ServerMessage, WorkerMessage},
        sessions::SessionEvent,
    };

    /// Build a bidirectional pair of `WorkerSocket`s wired to each other in
    /// memory. The first acts as the worker side; the second as the server
    /// side (so tests can act-as-server and inspect what the worker sent /
    /// hand back canned `ServerMessage`s).
    fn paired_sockets() -> (WorkerSocket<TestStream>, WorkerSocket<TestStream>) {
        let (w_tx, s_rx) =
            mpsc::unbounded::<std::result::Result<tungstenite::Message, tungstenite::Error>>();
        let (s_tx, w_rx) =
            mpsc::unbounded::<std::result::Result<tungstenite::Message, tungstenite::Error>>();
        let worker = WorkerSocket::new(TestStream::new(w_rx, w_tx));
        let server = WorkerSocket::new(TestStream::new(s_rx, s_tx));
        (worker, server)
    }

    /// A `Sink + Stream` duplex over `futures::channel::mpsc` used to mock a
    /// tungstenite WebSocket in tests.
    struct TestStream {
        rx: mpsc::UnboundedReceiver<std::result::Result<tungstenite::Message, tungstenite::Error>>,
        tx: mpsc::UnboundedSender<std::result::Result<tungstenite::Message, tungstenite::Error>>,
    }

    impl TestStream {
        fn new(
            rx: mpsc::UnboundedReceiver<
                std::result::Result<tungstenite::Message, tungstenite::Error>,
            >,
            tx: mpsc::UnboundedSender<
                std::result::Result<tungstenite::Message, tungstenite::Error>,
            >,
        ) -> Self {
            Self { rx, tx }
        }
    }

    impl Stream for TestStream {
        type Item = std::result::Result<tungstenite::Message, tungstenite::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::pin::Pin::new(&mut self.rx).poll_next(cx)
        }
    }

    impl Sink<tungstenite::Message> for TestStream {
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
    async fn round_trips_worker_event_and_server_catch_up() {
        let (mut worker, mut server) = paired_sockets();

        let event = SessionEvent::UserMessage {
            content: "hi".to_string(),
            timestamp: Utc::now(),
        };
        worker
            .send(WorkerMessage::Event {
                event: event.clone(),
            })
            .await
            .unwrap();

        let raw = server.inner.next().await.expect("frame").unwrap();
        match raw {
            tungstenite::Message::Text(text) => {
                let parsed: WorkerMessage = serde_json::from_str(&text).unwrap();
                match parsed {
                    WorkerMessage::Event { event: e } => assert_eq!(e, event),
                    other => panic!("expected Event, got {other:?}"),
                }
            }
            other => panic!("expected text frame, got {other:?}"),
        }

        let cu = ServerMessage::CatchUp { events: vec![] };
        server.send_raw_server_message(&cu).await;
        let got = worker.recv().await.unwrap().expect("server message");
        match got {
            ServerMessage::CatchUp { events } => assert!(events.is_empty()),
            other => panic!("expected CatchUp, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn round_trips_fresh_handshake() {
        let (mut worker, mut server) = paired_sockets();
        worker.send(WorkerMessage::Fresh).await.unwrap();
        let raw = server.inner.next().await.expect("frame").unwrap();
        let tungstenite::Message::Text(text) = raw else {
            panic!("expected text frame");
        };
        let parsed: WorkerMessage = serde_json::from_str(&text).unwrap();
        assert!(matches!(parsed, WorkerMessage::Fresh));
    }

    impl WorkerSocket<TestStream> {
        /// Test helper: push a `ServerMessage` onto the "server → worker"
        /// half of the duplex by serializing through the inner sink.
        async fn send_raw_server_message(&mut self, msg: &ServerMessage) {
            let json = serde_json::to_string(msg).unwrap();
            self.inner
                .send(tungstenite::Message::Text(json))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn recv_returns_none_on_close_frame() {
        let (mut worker, mut server) = paired_sockets();
        server
            .inner
            .send(tungstenite::Message::Close(None))
            .await
            .unwrap();
        assert!(worker.recv().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn recv_auto_pongs_pings_then_yields_next_text_frame() {
        let (mut worker, mut server) = paired_sockets();

        // Server pings, then sends an actual message.
        server
            .inner
            .send(tungstenite::Message::Ping(vec![1, 2, 3]))
            .await
            .unwrap();
        let cu = ServerMessage::CatchUp { events: vec![] };
        server.send_raw_server_message(&cu).await;

        let got = worker.recv().await.unwrap().expect("server message");
        assert!(matches!(got, ServerMessage::CatchUp { .. }));

        // Server side should have received an automatic Pong.
        let raw = server.inner.next().await.expect("pong frame").unwrap();
        assert!(matches!(raw, tungstenite::Message::Pong(p) if p == vec![1, 2, 3]));
    }
}
