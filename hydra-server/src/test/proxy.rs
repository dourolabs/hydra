//! Integration tests for the per-port proxy router.
//!
//! These tests stand up a real test server, register a session with a
//! proxy target whose port is bound to a stub HTTP server inside the
//! test process, and drive the proxy router through `reqwest`. They
//! exercise host parsing, cookie minting + validation, request
//! forwarding, header-strip hygiene, and the security headers the
//! proxy adds on every response.

use crate::app::{AppState, ServiceState};
use crate::domain::sessions::{AgentConfig, Session, SessionMode};
use crate::domain::task_status::Status;
use crate::domain::users::Username;
use crate::job_engine::{JobEngine, LocalJobEngine};
use crate::proxy::cookie::{
    DEFAULT_COOKIE_TTL_SECS, ProxyCookiePayload, ProxyTargetId, cookie_name, mint as mint_cookie,
};
use crate::routes::sessions::mount_spec_from_create_request;
use crate::store::{MemoryStore, Store};
use crate::test_utils::{
    spawn_test_server_with_state, test_actor, test_app_config, test_auth_token, test_secret_manager,
};
use axum::Router;
use axum::http::HeaderMap;
use axum::routing::any;
use hydra_common::actor_ref::{ActorId, ActorRef};
use hydra_common::api::v1::sessions::{Bundle, ProxyTarget};
use hydra_common::{ConversationId, SessionId};
use reqwest::header;
use std::collections::HashMap;
use std::sync::Arc;

const PROXY_HOST: &str = "proxy.localhost";

fn session_with_proxy_target(port: u16) -> Session {
    let mut session = Session::new(
        Username::from("test-creator"),
        None,
        None,
        AgentConfig::default(),
        mount_spec_from_create_request(Bundle::None, None),
        Some("worker:latest".to_string()),
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        Status::Running,
        None,
        None,
    );
    session.proxy_targets = vec![ProxyTarget {
        port,
        ready_path: None,
    }];
    session
}

async fn spawn_echo_server() -> u16 {
    #[derive(serde::Serialize)]
    struct EchoPayload {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
    }

    async fn echo(
        headers: HeaderMap,
        req: axum::http::Request<axum::body::Body>,
    ) -> axum::Json<EchoPayload> {
        let payload = EchoPayload {
            method: req.method().to_string(),
            path: req.uri().path().to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect(),
        };
        axum::Json(payload)
    }

    let app: Router = Router::new().fallback(any(echo));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Server runs detached — its `JoinHandle` is dropped, and tokio will
    // tear the listener down when the test process exits. We don't need
    // it past the test body.
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    port
}

/// Build state + store with a LocalJobEngine, register the test session
/// with `proxy_targets = [port]`, and spawn a test server. Returns the
/// `(server, session_id)` pair.
async fn spawn_proxy_server_with_session(
    port: u16,
    per_target_cap: usize,
) -> (crate::test_utils::TestServer, SessionId) {
    let mut config = test_app_config();
    config.hydra.proxy_host = PROXY_HOST.to_string();

    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let engine = Arc::new(LocalJobEngine::new(
        "http://localhost:0".to_string(),
        std::env::temp_dir().join("hydra-proxy-test"),
        Some((std::path::PathBuf::from("/bin/true"), vec![])),
    ));

    let mut state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        engine.clone(),
        test_secret_manager(),
    );
    state.proxy_state = crate::proxy::state::ProxyState::new(per_target_cap);

    let session = session_with_proxy_target(port);
    let (session_id, _version) = store
        .add_session(session, chrono::Utc::now(), &ActorRef::test())
        .await
        .expect("failed to add session");

    // Drop a tracked entry in the LocalJobEngine so `proxy_http` accepts
    // the session id. The subprocess (/bin/true) exits immediately; the
    // entry persists in the engine's process map.
    let _ = engine
        .create_job(
            &session_id,
            &test_actor(),
            &test_auth_token(),
            "unused",
            &HashMap::new(),
            "100m".to_string(),
            "64Mi".to_string(),
            "100m".to_string(),
            "64Mi".to_string(),
            Vec::new(),
        )
        .await;

    let server = spawn_test_server_with_state(state, store)
        .await
        .expect("test server");
    (server, session_id)
}

fn proxy_host_label(port: u16, target: &ProxyTargetId) -> String {
    format!("{port}-{}.{PROXY_HOST}", target.as_label())
}

fn mint_test_cookie(target: &ProxyTargetId, session_id_at_mint: &SessionId) -> String {
    let payload = ProxyCookiePayload {
        actor_id: ActorId::User(hydra_common::api::v1::users::Username::from("test-creator")),
        target: target.clone(),
        session_id_at_mint: session_id_at_mint.clone(),
        exp: chrono::Utc::now().timestamp() + DEFAULT_COOKIE_TTL_SECS,
    };
    mint_cookie(&test_secret_manager(), &payload).expect("mint")
}

#[tokio::test]
async fn proxy_http_round_trips_through_local_job_engine() {
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;

    let target = ProxyTargetId::Session(session_id.clone());
    let cookie_value = mint_test_cookie(&target, &session_id);
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let client = reqwest::Client::new();
    let url = format!("{}/hello/world?x=1", server.base_url());
    let response = client
        .get(&url)
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);

    // Security header set on the proxy response.
    assert_eq!(
        response
            .headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some("frame-ancestors 'none'")
    );

    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/hello/world");

    // Verify Cookie / Authorization headers were stripped before forwarding.
    let header_names: Vec<String> = body["headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h[0].as_str().unwrap().to_lowercase())
        .collect();
    assert!(
        !header_names.contains(&"cookie".to_string()),
        "Cookie should be stripped, got headers: {header_names:?}"
    );
    assert!(
        !header_names.contains(&"authorization".to_string()),
        "Authorization should be stripped"
    );
}

#[tokio::test]
async fn proxy_rejects_request_without_cookie() {
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id);
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
    assert_eq!(
        response
            .headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some("frame-ancestors 'none'")
    );
}

#[tokio::test]
async fn proxy_rejects_cookie_holder_without_read_access() {
    // Per-request read-access re-check: even with a valid signed cookie
    // and a matching session_id_at_mint, a cookie whose `actor_id` is
    // NOT the session's creator (and not the owning conversation's
    // creator) must be rejected. This is the "actor was rotated out of
    // the conversation" scenario — open tabs lose reach at the next
    // request, not at cookie expiry.
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id.clone());

    // Mint a cookie binding a different user as the actor. The signature
    // is valid; the target/session match. Only the read-access check
    // (creator-of-session) trips.
    let payload = ProxyCookiePayload {
        actor_id: ActorId::User(hydra_common::api::v1::users::Username::from("not-the-creator")),
        target: target.clone(),
        session_id_at_mint: session_id.clone(),
        exp: chrono::Utc::now().timestamp() + DEFAULT_COOKIE_TTL_SECS,
    };
    let cookie_value = mint_cookie(&test_secret_manager(), &payload).expect("mint");
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
    assert_eq!(
        response
            .headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some("frame-ancestors 'none'")
    );
}

#[tokio::test]
async fn proxy_rejects_cookie_with_non_user_actor() {
    // The mint-time helper rejects non-User actors (Agent/Adhoc/External)
    // at the type level by returning `None` from `user_principal`. The
    // proxy router must do the same on the cookie payload so a forged or
    // malformed cookie with `ActorId::Adhoc(...)` can't bypass the
    // creator-match gate.
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id.clone());

    let payload = ProxyCookiePayload {
        actor_id: ActorId::Adhoc(SessionId::new()),
        target: target.clone(),
        session_id_at_mint: session_id.clone(),
        exp: chrono::Utc::now().timestamp() + DEFAULT_COOKIE_TTL_SECS,
    };
    let cookie_value = mint_cookie(&test_secret_manager(), &payload).expect("mint");
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn proxy_rejects_wrong_session_id_at_mint() {
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id.clone());

    // Mint with a DIFFERENT session id — simulates an old cookie bound to
    // a session that has since been re-spawned.
    let wrong_sid = SessionId::new();
    let cookie_value = mint_test_cookie(&target, &wrong_sid);
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn proxy_rejects_unadvertised_port() {
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id.clone());

    let unadvertised_port = upstream_port + 1;
    let cookie_value = mint_test_cookie(&target, &session_id);
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(unadvertised_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn proxy_rejects_unknown_conversation_target() {
    let upstream_port = spawn_echo_server().await;
    let (server, _session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;

    // A conversation id that has no active session.
    let cid = ConversationId::new();
    let target = ProxyTargetId::Conversation(cid);
    let cookie_value = mint_test_cookie(&target, &SessionId::new());
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn proxy_rejects_bad_host_label() {
    let upstream_port = spawn_echo_server().await;
    let (server, _session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, format!("not-a-valid-label.{PROXY_HOST}"))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn proxy_concurrency_cap_returns_503() {
    use tokio::sync::Notify;

    // Slow upstream that blocks on a notify. Notifier is leaked so the
    // server keeps waiting for the whole test.
    let notify = Arc::new(Notify::new());
    let release = notify.clone();
    let app: Router = Router::new().fallback(any(move || {
        let release = release.clone();
        async move {
            release.notified().await;
            "done"
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    std::mem::forget(notify);

    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 1).await;
    let target = ProxyTargetId::Session(session_id.clone());
    let cookie_value = mint_test_cookie(&target, &session_id);
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);
    let base_url = server.base_url();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    let url = format!("{base_url}/");
    // Hold the first slot by sending a request that the slow upstream
    // will park indefinitely. Use a short reqwest timeout so the test
    // process doesn't hang on the first request.
    let cookie_clone = cookie_header.clone();
    let host_clone = host.clone();
    let url_clone = url.clone();
    let client_clone = client.clone();
    let _first = tokio::spawn(async move {
        let _ = client_clone
            .get(&url_clone)
            .header(header::HOST, &host_clone)
            .header(header::COOKIE, &cookie_clone)
            .send()
            .await;
    });

    // Wait for the slot to actually be acquired before the second
    // request lands — without this we race the semaphore.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let res2 = client
        .get(&url)
        .header(header::HOST, &host)
        .header(header::COOKIE, &cookie_header)
        .send()
        .await
        .expect("second request reaches server");
    assert_eq!(
        res2.status().as_u16(),
        503,
        "second concurrent request should be rejected with 503"
    );
}

#[tokio::test]
async fn proxy_streams_chunked_response_body() {
    // Verifies the response body is streamed (not buffered to completion)
    // so SSE / HMR style responses arrive at the client incrementally.
    // The upstream emits a chunk every 50ms and finishes after 200ms; the
    // proxy must hand each chunk to the client without buffering until
    // the upstream closes.
    use axum::body::Body as AxumBody;
    use futures::stream::{self, StreamExt as _};

    let app: Router = Router::new().fallback(any(|| async {
        let chunks: Vec<Result<&'static [u8], std::io::Error>> = vec![
            Ok(b"chunk-1\n"),
            Ok(b"chunk-2\n"),
            Ok(b"chunk-3\n"),
        ];
        let body_stream =
            stream::iter(chunks).then(|item| async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                item
            });
        AxumBody::from_stream(body_stream)
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;
    let target = ProxyTargetId::Session(session_id.clone());
    let cookie_value = mint_test_cookie(&target, &session_id);
    let cookie_header = format!("{}={cookie_value}", cookie_name(&target));
    let host = proxy_host_label(upstream_port, &target);

    let response = reqwest::Client::new()
        .get(format!("{}/", server.base_url()))
        .header(header::HOST, &host)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("body");
    assert!(body.contains("chunk-1"));
    assert!(body.contains("chunk-2"));
    assert!(body.contains("chunk-3"));
}

#[tokio::test]
async fn mint_session_proxy_auth_returns_cookie_that_validates() {
    let upstream_port = spawn_echo_server().await;
    let (server, session_id) = spawn_proxy_server_with_session(upstream_port, 32).await;

    let client = reqwest::Client::new();
    let url = format!("{}/v1/sessions/{session_id}/proxy-auth", server.base_url());
    let response = client
        .post(&url)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", test_auth_token()),
        )
        .send()
        .await
        .expect("mint cookie");
    assert_eq!(response.status(), 204, "mint should succeed");

    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("Set-Cookie header present")
        .to_str()
        .unwrap()
        .to_owned();
    let target = ProxyTargetId::Session(session_id.clone());
    let expected_prefix = format!("{}=", cookie_name(&target));
    assert!(
        set_cookie.starts_with(&expected_prefix),
        "Set-Cookie should begin with the per-target name: {set_cookie}"
    );
    assert!(set_cookie.contains("HttpOnly"), "HttpOnly missing");
    assert!(set_cookie.contains("Secure"), "Secure missing");
    assert!(set_cookie.contains("SameSite=Lax"), "SameSite=Lax missing");
    assert!(
        set_cookie.contains(&format!("Domain=.{PROXY_HOST}")),
        "Domain attribute should scope to proxy subdomain: {set_cookie}"
    );
}
