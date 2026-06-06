//! Per-port subdomain proxy router.
//!
//! Mounted on `Host: <port>-<HydraId>.proxy.<host>`. The router has its
//! own auth (cookie-based) and is wired as a fully separate axum
//! `Router` tree from the main API; the dispatch in `lib.rs::build_app`
//! decides which router handles each inbound request based on the Host
//! header.

use std::str::FromStr;

use axum::{
    Router,
    body::Body,
    extract::{State, ws::WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode, Uri, header},
    response::IntoResponse,
    routing::any,
};
use hydra_common::SessionId;
use tracing::{debug, info, warn};

use crate::{
    app::AppState,
    proxy::{
        access,
        cookie::{self, ProxyCookiePayload, ProxyTargetId},
        host::{HostParseError, parse as parse_host},
    },
};

/// Build the axum `Router` mounted on the proxy subdomain. The router has
/// no middleware shared with the main API — cookie auth is enforced inline
/// in the handler.
pub fn build_router() -> Router<AppState> {
    Router::new().fallback(any(proxy_entrypoint))
}

/// Single catch-all handler for the proxy subdomain. All host-label
/// parsing, cookie validation, target/port resolution, and engine
/// dispatch happens here.
async fn proxy_entrypoint(
    State(state): State<AppState>,
    upgrade: Option<WebSocketUpgrade>,
    req: Request<Body>,
) -> Response<Body> {
    match handle(state, upgrade, req).await {
        Ok(mut response) => {
            set_security_headers(response.headers_mut());
            response
        }
        Err(rejection) => {
            let (status, body) = rejection.into_parts();
            let mut response = (status, body).into_response();
            set_security_headers(response.headers_mut());
            response
        }
    }
}

/// Response composer: forward to upstream, apply security headers, ensure
/// the response body matches axum's `Body`.
async fn handle(
    state: AppState,
    upgrade: Option<WebSocketUpgrade>,
    req: Request<Body>,
) -> Result<Response<Body>, ProxyRejection> {
    let host_header = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ProxyRejection::bad_request("missing or non-ascii Host header"))?
        .to_owned();
    debug!(host = %host_header, method = %req.method(), uri = %req.uri(), "proxy request received");

    let proxy_suffix = &state.config.hydra.proxy_host;
    let parsed = parse_host(&host_header, proxy_suffix).map_err(|err| match err {
        HostParseError::Missing => ProxyRejection::bad_request("Host header is empty"),
        HostParseError::SuffixMismatch { host, suffix } => ProxyRejection::not_found(format!(
            "host '{host}' is not within the proxy suffix '.{suffix}'"
        )),
        HostParseError::BadLabel { label }
        | HostParseError::BadPort { label }
        | HostParseError::BadId { label } => {
            ProxyRejection::bad_request(format!("invalid proxy host label '{label}'"))
        }
    })?;

    // Resolve the target id to a SessionId. For Session targets, that's
    // direct. For Conversation targets, look up the currently-active
    // session via the chat relay map; if there is none, return 404 so
    // the UI prompts the user to send a message and re-activate.
    let resolved_session_id: SessionId = match &parsed.target {
        ProxyTargetId::Session(s) => s.clone(),
        ProxyTargetId::Conversation(c) => {
            state.chat_relay_map.active_session_id(c).ok_or_else(|| {
                ProxyRejection::not_found(format!(
                    "conversation '{c}' has no active session; send a message to resume"
                ))
            })?
        }
    };

    let session = state.get_session(&resolved_session_id).await.map_err(|_| {
        ProxyRejection::not_found(format!("session '{resolved_session_id}' not found"))
    })?;

    // Port-allowlist check: only ports the worker explicitly advertised
    // are reachable. Other ports on the same pod stay invisible.
    if !session.proxy_targets.iter().any(|t| t.port == parsed.port) {
        return Err(ProxyRejection::not_found(format!(
            "port {} is not advertised by session '{}'",
            parsed.port, resolved_session_id
        )));
    }

    // Cookie auth.
    let cookie_name = cookie::cookie_name(&parsed.target);
    let cookie_value = read_cookie(req.headers(), &cookie_name)
        .ok_or_else(|| ProxyRejection::unauthorized(format!("missing cookie '{cookie_name}'")))?;
    let payload: ProxyCookiePayload = cookie::decode(&state.secret_manager, &cookie_value)
        .map_err(|e| ProxyRejection::unauthorized(format!("invalid proxy cookie: {e}")))?;
    let now = chrono::Utc::now().timestamp();
    cookie::validate(&payload, &parsed.target, &resolved_session_id, now)
        .map_err(|e| ProxyRejection::unauthorized(format!("proxy cookie rejected: {e}")))?;

    // Re-check read access on every proxy request (not just at cookie
    // mint). Revoking a principal's membership — removing them from the
    // owning conversation, deleting their session, leaking the cookie to
    // an actor with a different `ActorId` — invalidates open tabs at the
    // next request rather than waiting for cookie expiry.
    if !access::has_read_access(&state, &payload.actor_id, &parsed.target, &session).await {
        warn!(
            actor = %payload.actor_id,
            target = %parsed.target,
            "proxy cookie holder no longer has read access; rejecting"
        );
        return Err(ProxyRejection::unauthorized(
            "actor no longer has read access to the proxy target".to_string(),
        ));
    }

    // Per-target concurrency cap. We hold the permit for the lifetime of
    // this request via the `_permit` binding — both HTTP and WS requests
    // count as one in-flight slot.
    let _permit = state
        .proxy_state
        .try_acquire(&parsed.target)
        .map_err(|_| ProxyRejection::overloaded(parsed.target.to_string()))?;

    // Forwarding hygiene: strip `Cookie` and `Authorization` so the dev
    // process never sees the user's auth state.
    let mut forwarded_req = strip_sensitive_headers(req);

    // WebSocket upgrade path: hand off the upgrade-bearing extractor and
    // the concurrency permit to the engine. The permit rides into the
    // spawned `on_upgrade` task so its `Drop` runs when the bidirectional
    // pump exits — an open WS connection keeps occupying its per-target
    // slot for as long as the socket lives.
    if let Some(upgrade) = upgrade {
        info!(
            session_id = %resolved_session_id,
            port = parsed.port,
            "proxying WebSocket upgrade"
        );
        let pump_guard: crate::job_engine::WsPumpGuard = Box::new(_permit);
        let response = state
            .job_engine
            .proxy_ws(&resolved_session_id, parsed.port, upgrade, pump_guard)
            .await
            .map_err(|err| ProxyRejection::bad_gateway(format!("proxy_ws: {err}")))?;
        return Ok(response);
    }

    // Rewrite the URI so the upstream sees a fresh scheme/authority
    // pointing at it (the proxy helper rebuilds the URL anyway, but
    // axum's request URI was populated from the inbound `Host` and we
    // don't want the proxy-subdomain hostname leaking into upstream
    // logs).
    forwarded_req = rewrite_uri_for_upstream(forwarded_req)?;

    info!(
        session_id = %resolved_session_id,
        port = parsed.port,
        method = %forwarded_req.method(),
        path = %forwarded_req.uri().path(),
        "proxying HTTP request"
    );

    let response = state
        .job_engine
        .proxy_http(&resolved_session_id, parsed.port, forwarded_req)
        .await
        .map_err(|err| ProxyRejection::bad_gateway(format!("proxy_http: {err}")))?;

    Ok(response)
}

fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for kv in raw.split(';') {
        let kv = kv.trim();
        if let Some((k, v)) = kv.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn strip_sensitive_headers(mut req: Request<Body>) -> Request<Body> {
    let headers = req.headers_mut();
    headers.remove(header::COOKIE);
    headers.remove(header::AUTHORIZATION);
    req
}

fn rewrite_uri_for_upstream(mut req: Request<Body>) -> Result<Request<Body>, ProxyRejection> {
    // Re-build a path+query-only URI so the proxy helper doesn't carry
    // the original `Host:port` authority.
    let pq = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_owned())
        .unwrap_or_else(|| "/".to_owned());
    let uri =
        Uri::from_str(&pq).map_err(|e| ProxyRejection::bad_request(format!("rewrite uri: {e}")))?;
    *req.uri_mut() = uri;
    Ok(req)
}

/// Add headers that always apply on proxy responses, regardless of what
/// the upstream sent or whether we are returning an error of our own.
fn set_security_headers(headers: &mut HeaderMap) {
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    // Defense-in-depth: stop the user's content from being rendered by an
    // ancient Internet-Explorer-style framing mechanism.
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
}

/// Internal rejection type — funnels errors through a single sink that
/// always attaches the security headers.
#[derive(Debug)]
struct ProxyRejection {
    status: StatusCode,
    message: String,
}

impl ProxyRejection {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: msg.into(),
        }
    }
    fn bad_gateway(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: msg.into(),
        }
    }
    fn overloaded(target: String) -> Self {
        warn!(target = %target, "per-target proxy concurrency cap exhausted");
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: format!("proxy cap exhausted for target '{target}'"),
        }
    }
    fn into_parts(self) -> (StatusCode, String) {
        (self.status, self.message)
    }
}

/// Decide whether a request's `Host` header points at the proxy
/// subdomain. The dispatch wrapper in `lib.rs` uses this to choose
/// between the main API router and the proxy router.
pub fn host_matches_proxy(host_header: &str, proxy_suffix: &str) -> bool {
    if host_header.is_empty() || proxy_suffix.is_empty() {
        return false;
    }
    let host_no_port = host_header
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_header);
    let dotted = format!(".{proxy_suffix}");
    host_no_port.ends_with(&dotted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn host_matches_proxy_accepts_subdomains() {
        assert!(host_matches_proxy(
            "3000-c-abc.proxy.localhost",
            "proxy.localhost"
        ));
        assert!(host_matches_proxy(
            "3000-c-abc.proxy.localhost:8080",
            "proxy.localhost"
        ));
        assert!(host_matches_proxy(
            "3000-s-xyz.proxy.example.com",
            "proxy.example.com"
        ));
    }

    #[test]
    fn host_matches_proxy_rejects_main_host() {
        assert!(!host_matches_proxy(
            "hydra.example.com",
            "proxy.example.com"
        ));
        assert!(!host_matches_proxy("localhost", "proxy.localhost"));
        assert!(!host_matches_proxy("", "proxy.localhost"));
    }

    #[test]
    fn host_matches_proxy_rejects_exact_suffix() {
        // The bare suffix is not a proxy host (no `.<port>-<id>` prefix).
        assert!(!host_matches_proxy("proxy.localhost", "proxy.localhost"));
    }

    #[test]
    fn read_cookie_extracts_named_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("foo=bar; hydra_proxy_AAAA=token123; baz=qux"),
        );
        assert_eq!(
            read_cookie(&headers, "hydra_proxy_AAAA").as_deref(),
            Some("token123")
        );
        assert_eq!(read_cookie(&headers, "missing"), None);
    }

    #[test]
    fn read_cookie_returns_none_when_header_missing() {
        let headers = HeaderMap::new();
        assert_eq!(read_cookie(&headers, "hydra_proxy_AAAA"), None);
    }

    #[test]
    fn strip_sensitive_headers_removes_cookie_and_authorization() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .header(header::COOKIE, "hydra_proxy_xyz=tok")
            .header(header::AUTHORIZATION, "Bearer abc")
            .header("x-other", "passthrough")
            .body(Body::empty())
            .unwrap();
        let stripped = strip_sensitive_headers(req);
        assert!(stripped.headers().get(header::COOKIE).is_none());
        assert!(stripped.headers().get(header::AUTHORIZATION).is_none());
        assert_eq!(
            stripped.headers().get("x-other").unwrap().to_str().unwrap(),
            "passthrough"
        );
    }

    #[test]
    fn set_security_headers_adds_csp_and_frame_options() {
        let mut headers = HeaderMap::new();
        set_security_headers(&mut headers);
        assert_eq!(
            headers
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "frame-ancestors 'none'"
        );
        assert_eq!(
            headers.get("x-frame-options").unwrap().to_str().unwrap(),
            "DENY"
        );
    }
}
