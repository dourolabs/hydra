use axum::{
    body::Body,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use futures::{SinkExt, StreamExt};
use http::Request;

use crate::state::BffState;
use crate::upstream::Upstream;

/// WebSocket relay proxy for `/v1/sessions/:session_id/relay`.
/// Preserves the existing Authorization header (direct pass-through).
pub async fn v1_ws_relay<U: Upstream>(
    State(bff): State<BffState<U>>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    ws: WebSocketUpgrade,
    request: Request<Body>,
) -> Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    ws_relay_inner(bff, &session_id, auth_header, ws)
}

/// WebSocket relay proxy for `/api/v1/sessions/:session_id/relay`.
/// Translates cookie auth to Bearer token.
pub async fn api_ws_relay<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    ws: WebSocketUpgrade,
    _request: Request<Body>,
) -> impl IntoResponse {
    let token = match bff.resolve_token(&jar) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "not authenticated" })),
            )
                .into_response();
        }
    };

    ws_relay_inner(bff, &session_id, Some(format!("Bearer {token}")), ws).into_response()
}

fn ws_relay_inner<U: Upstream>(
    bff: BffState<U>,
    session_id: &str,
    auth_header: Option<String>,
    ws: WebSocketUpgrade,
) -> Response {
    let ws_base = match bff.upstream.ws_base_url() {
        Some(url) => url,
        None => {
            // InProcessUpstream: fall through to the regular proxy which handles
            // WebSocket via oneshot. This path should not be hit in practice since
            // InProcessUpstream routes are handled by the inner router directly.
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": "WebSocket proxy not available for in-process upstream" })),
            )
                .into_response();
        }
    };

    let upstream_url = format!(
        "{}/v1/sessions/{}/relay",
        ws_base.trim_end_matches('/'),
        session_id
    );

    let session_id = session_id.to_string();

    tracing::info!(
        %session_id,
        %upstream_url,
        "proxying WebSocket relay to upstream"
    );

    ws.on_upgrade(move |client_socket| {
        bridge_websockets(client_socket, upstream_url, auth_header, session_id)
    })
}

async fn bridge_websockets(
    client_socket: WebSocket,
    upstream_url: String,
    auth_header: Option<String>,
    session_id: String,
) {
    // Build the upstream WebSocket connection request with auth headers.
    let mut request = http::Request::builder().uri(&upstream_url).header(
        "Host",
        http::Uri::try_from(&upstream_url)
            .ok()
            .and_then(|u| u.host().map(|h| h.to_string()))
            .unwrap_or_default(),
    );

    if let Some(auth) = &auth_header {
        request = request.header(header::AUTHORIZATION, auth);
    }

    let request = match request.body(()) {
        Ok(req) => req,
        Err(err) => {
            tracing::error!(%session_id, error = %err, "failed to build upstream WebSocket request");
            return;
        }
    };

    let (upstream_ws, _response) = match tokio_tungstenite::connect_async(request).await {
        Ok(conn) => conn,
        Err(err) => {
            tracing::error!(%session_id, error = %err, "failed to connect to upstream WebSocket");
            return;
        }
    };

    tracing::info!(%session_id, "upstream WebSocket connected, starting relay bridge");

    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream_ws.split();

    // Client -> Upstream
    let session_id_c2u = session_id.clone();
    let client_to_upstream = async move {
        while let Some(msg) = client_rx.next().await {
            match msg {
                Ok(msg) => {
                    let tung_msg = axum_msg_to_tungstenite(msg);
                    if let Some(tung_msg) = tung_msg {
                        if upstream_tx.send(tung_msg).await.is_err() {
                            tracing::debug!(session_id = %session_id_c2u, "upstream WebSocket closed");
                            break;
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(session_id = %session_id_c2u, error = %err, "client WebSocket error");
                    break;
                }
            }
        }
    };

    // Upstream -> Client
    let session_id_u2c = session_id.clone();
    let upstream_to_client = async move {
        while let Some(msg) = upstream_rx.next().await {
            match msg {
                Ok(msg) => {
                    let axum_msg = tungstenite_msg_to_axum(msg);
                    if let Some(axum_msg) = axum_msg {
                        if client_tx.send(axum_msg).await.is_err() {
                            tracing::debug!(session_id = %session_id_u2c, "client WebSocket closed");
                            break;
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(session_id = %session_id_u2c, error = %err, "upstream WebSocket error");
                    break;
                }
            }
        }
    };

    // Run both directions concurrently; when either finishes, the relay is done.
    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }

    tracing::info!(%session_id, "WebSocket relay bridge closed");
}

/// Convert an axum WebSocket message to a tungstenite message.
fn axum_msg_to_tungstenite(msg: Message) -> Option<tokio_tungstenite::tungstenite::Message> {
    use tokio_tungstenite::tungstenite::Message as TMsg;
    match msg {
        Message::Text(text) => Some(TMsg::Text(text)),
        Message::Binary(data) => Some(TMsg::Binary(data)),
        Message::Ping(data) => Some(TMsg::Ping(data)),
        Message::Pong(data) => Some(TMsg::Pong(data)),
        Message::Close(frame) => Some(TMsg::Close(frame.map(|f| {
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: f.code.into(),
                reason: f.reason,
            }
        }))),
    }
}

/// Convert a tungstenite message to an axum WebSocket message.
fn tungstenite_msg_to_axum(msg: tokio_tungstenite::tungstenite::Message) -> Option<Message> {
    use tokio_tungstenite::tungstenite::Message as TMsg;
    match msg {
        TMsg::Text(text) => Some(Message::Text(text)),
        TMsg::Binary(data) => Some(Message::Binary(data)),
        TMsg::Ping(data) => Some(Message::Ping(data)),
        TMsg::Pong(data) => Some(Message::Pong(data)),
        TMsg::Close(frame) => Some(Message::Close(frame.map(|f| {
            axum::extract::ws::CloseFrame {
                code: f.code.into(),
                reason: f.reason,
            }
        }))),
        TMsg::Frame(_) => None,
    }
}
