//! Shared HTTP/WebSocket forwarding helpers for `JobEngine::proxy_http` /
//! `proxy_ws` impls.
//!
//! Each impl resolves a runtime-local upstream host (pod IP / container IP /
//! `localhost`) and delegates request body + headers forwarding to the helpers
//! here. The helpers don't know about sessions, ports, or auth — they just
//! relay bytes.

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

/// Forward an HTTP request to `http://<host>:<port>`, returning the upstream
/// response as-is. The caller has already stripped any request headers that
/// must not leak (e.g. `Cookie`, `Authorization`).
///
/// Uses `reqwest` (which we already depend on for outbound HTTP) so we don't
/// pull in another HTTP client. The connection is short-lived and not pooled
/// across calls — the proxy router caps concurrency at its own layer.
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

    // 16 MiB is comfortably above any reasonable dev-server response body
    // (HMR diffs, JSON, image responses); higher limits invite OOM if a
    // misbehaving upstream returns multi-GiB. Picking a cap is judgment;
    // 16 MiB matches what we use elsewhere for bulk-document handling.
    const MAX_PROXY_BODY_BYTES: usize = 16 * 1024 * 1024;
    let body_bytes = to_bytes(body, MAX_PROXY_BODY_BYTES)
        .await
        .map_err(|e| ProxyError::Transport(format!("read request body: {e}")))?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ProxyError::Transport(format!("build client: {e}")))?;

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
    let body_bytes = upstream
        .bytes()
        .await
        .map_err(|e| ProxyError::Transport(format!("upstream body: {e}")))?;

    let mut response = Response::builder().status(
        StatusCode::from_u16(status.as_u16())
            .map_err(|e| ProxyError::Transport(format!("status code: {e}")))?,
    );
    if let Some(headers_mut) = response.headers_mut() {
        for (name, value) in headers.iter() {
            // Drop hop-by-hop headers that don't make sense to re-emit.
            if matches!(
                name.as_str(),
                "connection"
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

    response
        .body(Body::from(body_bytes))
        .map_err(|e| ProxyError::Transport(format!("build response: {e}")))
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
pub async fn pump_ws_frames(
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
