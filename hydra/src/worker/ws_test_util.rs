//! Shared WS-duplex test scaffolding for worker-side unit tests.
//!
//! Both `model_selector::tests::unified_cleanup` and
//! `relay_adapter::tests` need a way to stand up a `WorkerSocket` backed
//! by in-memory channels so they can drive both directions without a
//! real network. This module hosts the common pieces: the `TestStream`
//! type, the `duplex()` constructor, and helpers for pushing/collecting
//! frames.

use futures::SinkExt;
use hydra_common::api::v1::conversations::{ServerMessage, WorkerMessage};
use tokio_tungstenite::tungstenite;

use crate::worker::socket::WorkerSocket;

pub type WsFrame = std::result::Result<tungstenite::Message, tungstenite::Error>;
pub type WsSender = futures::channel::mpsc::UnboundedSender<WsFrame>;
pub type WsReceiver = futures::channel::mpsc::UnboundedReceiver<WsFrame>;

/// A minimal `Sink<Message> + Stream<Item = Result<Message, Error>>`
/// duplex over `futures::channel::mpsc`. Wraps two unbounded channels:
/// the receiver carries inbound (server-pushed) frames, the sender
/// emits outbound (worker-written) frames.
pub struct TestStream {
    rx: futures::channel::mpsc::UnboundedReceiver<WsFrame>,
    tx: futures::channel::mpsc::UnboundedSender<WsFrame>,
}

impl futures::Stream for TestStream {
    type Item = WsFrame;
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

/// Build a `WorkerSocket` paired with the server-side sender/receiver.
/// The returned tuple is `(ws, server_tx, server_rx)`:
/// * `ws` — the worker uses this for `send` / `recv`.
/// * `server_tx` — push inbound frames to simulate server messages.
/// * `server_rx` — read outbound frames the worker sent.
pub fn duplex() -> (WorkerSocket<TestStream>, WsSender, WsReceiver) {
    let (server_tx, worker_rx) = futures::channel::mpsc::unbounded::<WsFrame>();
    let (worker_tx, server_rx) = futures::channel::mpsc::unbounded::<WsFrame>();
    let ws = WorkerSocket::new(TestStream {
        rx: worker_rx,
        tx: worker_tx,
    });
    (ws, server_tx, server_rx)
}

/// Serialize a `ServerMessage` and push it as a text frame onto the
/// server-side sender so the worker observes it as inbound.
pub async fn push_server_msg(server_tx: &mut WsSender, msg: &ServerMessage) {
    let json = serde_json::to_string(msg).unwrap();
    server_tx
        .send(Ok(tungstenite::Message::Text(json)))
        .await
        .unwrap();
}

/// Drain every `WorkerMessage` the worker wrote until the server-side
/// receiver closes. Non-`Text` and non-deserializable frames are
/// silently dropped.
pub async fn collect_worker_msgs(server_rx: &mut WsReceiver) -> Vec<WorkerMessage> {
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(Ok(frame)) = server_rx.next().await {
        if let tungstenite::Message::Text(text) = frame {
            if let Ok(msg) = serde_json::from_str::<WorkerMessage>(&text) {
                out.push(msg);
            }
        }
    }
    out
}
