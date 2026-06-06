//! Shared HTTP/WebSocket forwarding helpers for `JobEngine::proxy_http` /
//! `proxy_ws` impls.
//!
//! Each impl resolves a runtime-local upstream host (pod IP / container IP /
//! `localhost`) and delegates request body + headers forwarding to the helpers
//! here. The helpers don't know about sessions, ports, or auth — they just
//! relay bytes.

use std::sync::OnceLock;

use axum::body::Body;
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::http::{Request, Response, StatusCode};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tracing::{debug, warn};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The requested session/port is not reachable — pod missing, container
    /// not running, target IP unknown.
    #[error("upstream unreachable: {0}")]
    Unreachable(String),
    /// The transport layer to the upstream failed (TCP refused, TLS error,
    /// hyper protocol error).
    #[error("upstream transport error: {0}")]
    Transport(String),
    /// The implementation is not supported in this build (e.g. a feature
    /// flag is off).
    #[error("proxy not supported: {0}")]
    Unsupported(String),
}

/// Request-body cap. Buffered up-front (in contrast to the response, which
/// streams) because reqwest needs `Content-Length` figured out and most
/// dev-server request bodies are small. 16 MiB is well above any HMR-style
/// or JSON-API call — agents pushing larger payloads should hit the
/// dev-server directly, not through the proxy.
const MAX_PROXY_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// Hard cap on response body bytes we'll relay through the proxy. The
/// response is streamed (so this doesn't buffer), but a runaway upstream
/// can still saturate the proxy's network and disk; cut it off rather
/// than relay an unbounded byte count. 256 MiB covers any reasonable
/// dev-server asset response with significant headroom.
const MAX_PROXY_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;

/// Long-lived `reqwest::Client` shared across every proxy_http call.
/// Building a new client per call (the prior shape) rebuilt the TLS
/// config, connection pool, and DNS resolver each time — for an HMR
/// loop hitting the proxy hundreds of times per page that compounds
/// into noticeable latency and resource churn. One client, all calls.
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("default reqwest::Client::builder should succeed")
    })
}

/// Forward an HTTP request to `http://<host>:<port>`, streaming the
/// upstream response back to the client. The caller has already
/// stripped any request headers that must not leak (e.g. `Cookie`,
/// `Authorization`).
///
/// Response body is streamed via `bytes_stream() + Body::from_stream`,
/// so SSE / chunked responses arrive at the client as they arrive at
/// the proxy rather than being buffered to completion. A 256 MiB total
/// cap protects against pathological upstreams.
pub async fn proxy_http_to_upstream(
    host: &str,
    port: u16,
    req: Request<Body>,
) -> Result<Response<Body>, ProxyError> {
    use axum::body::to_bytes;

    let (parts, body) = req.into_parts();

    // Reuse the request path + query as-is; the upstream sees the same
    // URL the proxy was hit with (modulo the host header).
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target_url = format!("http://{host}:{port}{path_and_query}");

    let body_bytes = to_bytes(body, MAX_PROXY_REQUEST_BYTES)
        .await
        .map_err(|e| ProxyError::Transport(format!("read request body: {e}")))?;

    let client = shared_client();

    let mut builder = client.request(parts.method.clone(), &target_url);
    for (name, value) in parts.headers.iter() {
        // `Host` is rewritten by reqwest based on the target URL. Forwarding
        // the original would point the upstream at the proxy subdomain,
        // which dev servers like Vite reject as a "Host header" check
        // failure.
        if name == axum::http::header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    debug!(
        method = %parts.method,
        url = %target_url,
        "proxying HTTP to upstream"
    );

    let upstream = builder
        .send()
        .await
        .map_err(|e| ProxyError::Transport(format!("upstream send: {e}")))?;

    let status = upstream.status();
    let headers = upstream.headers().clone();

    let mut response = Response::builder().status(
        StatusCode::from_u16(status.as_u16())
            .map_err(|e| ProxyError::Transport(format!("status code: {e}")))?,
    );
    if let Some(headers_mut) = response.headers_mut() {
        for (name, value) in headers.iter() {
            // Drop hop-by-hop headers that don't make sense to re-emit.
            // `content-length` is also dropped because we're streaming —
            // hyper will re-encode with `transfer-encoding: chunked` so
            // a stale `content-length` would mismatch the framing.
            if matches!(
                name.as_str(),
                "connection"
                    | "content-length"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailers"
                    | "transfer-encoding"
                    | "upgrade"
            ) {
                continue;
            }
            headers_mut.insert(name, value.clone());
        }
    }

    let capped = capped_response_stream(upstream.bytes_stream(), MAX_PROXY_RESPONSE_BYTES);
    response
        .body(Body::from_stream(capped))
        .map_err(|e| ProxyError::Transport(format!("build response: {e}")))
}

/// Wrap a byte stream so it emits an error once `cap` total bytes have
/// flowed through. axum surfaces the stream error as a broken response
/// to the client; the upstream connection is dropped at the same time
/// because `bytes_stream`'s `Drop` aborts the inner reqwest request.
fn capped_response_stream<S>(
    stream: S,
    cap: u64,
) -> impl futures::Stream<Item = Result<bytes::Bytes, ProxyStreamError>> + Send + 'static
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
{
    use futures::StreamExt;
    let mut sent: u64 = 0;
    stream.map(move |chunk| match chunk {
        Ok(bytes) => {
            sent = sent.saturating_add(bytes.len() as u64);
            if sent > cap {
                Err(ProxyStreamError::CapExceeded { cap })
            } else {
                Ok(bytes)
            }
        }
        Err(e) => Err(ProxyStreamError::Upstream(e.to_string())),
    })
}

/// Streamed-body error surfaced inside the response stream so axum can
/// terminate the response cleanly. The byte cap is fatal to the
/// in-flight response only — subsequent requests are independent.
#[derive(Debug, thiserror::Error)]
pub enum ProxyStreamError {
    #[error("proxy response exceeded {cap} bytes")]
    CapExceeded { cap: u64 },
    #[error("upstream stream error: {0}")]
    Upstream(String),
}

/// Open a WebSocket upgrade to `ws://<host>:<port><path>` and relay frames
/// between it and the axum-side client `WebSocket`. The `upgrade.on_upgrade`
/// closure handles the bidirectional pump in a background task.
pub async fn proxy_ws_to_upstream(
    host: &str,
    port: u16,
    path_and_query: &str,
    upgrade: WebSocketUpgrade,
) -> Result<Response<Body>, ProxyError> {
    let target_url = format!("ws://{host}:{port}{path_and_query}");

    debug!(url = %target_url, "proxying WebSocket to upstream");

    // Open the upstream connection BEFORE accepting the client upgrade so
    // the client gets a sensible error (502) if the dev server isn't
    // serving WS on that port.
    let (upstream, _resp) = tokio_tungstenite::connect_async(&target_url)
        .await
        .map_err(|e| ProxyError::Transport(format!("upstream WS connect: {e}")))?;

    Ok(upgrade.on_upgrade(move |client_ws| async move {
        pump_ws_frames(client_ws, upstream).await;
    }))
}

/// Bidirectional frame pump between an axum `WebSocket` and a
/// `tokio_tungstenite::WebSocketStream`. Exits when either side closes.
async fn pump_ws_frames(
    client_ws: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let client_to_upstream = async {
        while let Some(msg) = client_rx.next().await {
            match msg {
                Ok(AxumMessage::Text(t)) => {
                    if up_tx.send(TungsteniteMessage::Text(t)).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Binary(b)) => {
                    if up_tx.send(TungsteniteMessage::Binary(b)).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Ping(p)) => {
                    if up_tx.send(TungsteniteMessage::Ping(p)).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Pong(p)) => {
                    if up_tx.send(TungsteniteMessage::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Ok(AxumMessage::Close(_)) => {
                    let _ = up_tx.send(TungsteniteMessage::Close(None)).await;
                    break;
                }
                Err(err) => {
                    warn!(error = %err, "client WS frame error");
                    break;
                }
            }
        }
    };

    let upstream_to_client = async {
        while let Some(msg) = up_rx.next().await {
            match msg {
                Ok(TungsteniteMessage::Text(t)) => {
                    if client_tx.send(AxumMessage::Text(t)).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Binary(b)) => {
                    if client_tx.send(AxumMessage::Binary(b)).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Ping(p)) => {
                    if client_tx.send(AxumMessage::Ping(p)).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Pong(p)) => {
                    if client_tx.send(AxumMessage::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Ok(TungsteniteMessage::Close(_)) => {
                    let _ = client_tx.send(AxumMessage::Close(None)).await;
                    break;
                }
                Ok(TungsteniteMessage::Frame(_)) => {}
                Err(err) => {
                    warn!(error = %err, "upstream WS frame error");
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }
}
