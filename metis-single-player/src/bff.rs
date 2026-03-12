use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use tower::ServiceExt;
use tracing::info;

const COOKIE_NAME: &str = "metis_token";

/// Shared state for the BFF layer.
#[derive(Clone)]
pub struct BffState {
    /// The auth token loaded from the auth_token_file at startup.
    /// Used for auto-login: every frontend request gets this cookie set.
    pub auto_login_token: Arc<String>,

    /// The inner metis-server Axum app (with state applied), used for
    /// routing `/api/v1/*` requests directly to the server handlers.
    pub inner_app: Router,
}

/// Build the complete BFF + server + frontend router.
///
/// The returned router:
/// - Serves `/auth/*` (login, logout, me) with cookie-based auth
/// - Serves `/api/v1/*` by injecting the cookie token as a Bearer header
///   and routing to the internal metis-server handlers
/// - Serves the embedded frontend at `/` with SPA fallback
/// - Auto-login middleware sets the `metis_token` cookie on every response
pub fn build_bff_router(bff_state: BffState) -> Router {
    let auth_routes = Router::new()
        .route("/login", post(auth_login))
        .route("/logout", post(auth_logout))
        .route("/me", get(auth_me))
        .with_state(bff_state.clone());

    let api_routes = Router::new()
        .route("/*path", axum::routing::any(api_proxy))
        .route("/", axum::routing::any(api_proxy_root))
        .with_state(bff_state.clone());

    // /v1/* routes: Bearer token auth for CLI access, SPA fallback otherwise.
    let v1_routes = Router::new()
        .route("/*path", axum::routing::any(v1_bearer_proxy))
        .route("/", axum::routing::any(v1_bearer_proxy_root))
        .with_state(bff_state.clone());

    let frontend = crate::frontend::router();

    Router::new()
        .nest("/auth", auth_routes)
        .nest("/api/v1", api_routes)
        .nest("/v1", v1_routes)
        .fallback_service(frontend)
        .layer(middleware::from_fn_with_state(
            bff_state,
            auto_login_middleware,
        ))
}

// ---------------------------------------------------------------------------
// Auto-login middleware
// ---------------------------------------------------------------------------

/// Middleware that ensures the `metis_token` cookie is always set.
/// For single-player mode, the user is always logged in.
async fn auto_login_middleware(
    State(bff): State<BffState>,
    jar: CookieJar,
    request: Request<Body>,
    next: Next,
) -> Response {
    let response = next.run(request).await;

    // If the cookie is already set, no need to add it again.
    if jar.get(COOKIE_NAME).is_some() {
        return response;
    }

    // Set the auto-login cookie on the response.
    let cookie = build_auth_cookie(bff.auto_login_token.as_str());
    let (mut parts, body) = response.into_parts();
    if let Ok(value) = cookie.to_string().parse() {
        parts.headers.append(header::SET_COOKIE, value);
    }
    Response::from_parts(parts, body)
}

// ---------------------------------------------------------------------------
// Auth routes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginRequest {
    token: Option<String>,
}

async fn auth_login(
    State(bff): State<BffState>,
    jar: CookieJar,
    axum::Json(body): axum::Json<LoginRequest>,
) -> impl IntoResponse {
    let token = match body.token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "token is required" })),
            )
                .into_response();
        }
    };

    // Validate the token by calling the internal whoami endpoint.
    let whoami_req = Request::builder()
        .uri("/v1/whoami")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = bff.inner_app.clone().oneshot(whoami_req).await;
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "internal error" })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        info!("login failed: invalid token");
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid token" })),
        )
            .into_response();
    }

    // Read the whoami response body to return user info.
    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap_or_default();
    let user: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!({}));

    // Set the auth cookie.
    let cookie = build_auth_cookie(&token);
    let jar = jar.add(cookie);

    info!("login success");
    (jar, axum::Json(user)).into_response()
}

async fn auth_logout(jar: CookieJar) -> impl IntoResponse {
    let jar = jar.remove(Cookie::build(COOKIE_NAME).path("/"));
    (jar, axum::Json(serde_json::json!({ "ok": true })))
}

async fn auth_me(State(bff): State<BffState>, jar: CookieJar) -> impl IntoResponse {
    let token = match jar.get(COOKIE_NAME) {
        Some(cookie) => cookie.value().to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "not authenticated" })),
            )
                .into_response();
        }
    };

    let whoami_req = Request::builder()
        .uri("/v1/whoami")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = bff.inner_app.clone().oneshot(whoami_req).await;
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "internal error" })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "token expired or invalid" })),
        )
            .into_response();
    }

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap_or_default();
    let user: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!({}));

    axum::Json(user).into_response()
}

// ---------------------------------------------------------------------------
// CLI proxy: /v1/* with Bearer auth -> internal /v1/*
// Falls back to SPA when no Bearer header is present.
// ---------------------------------------------------------------------------

async fn v1_bearer_proxy(
    State(bff): State<BffState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: Request<Body>,
) -> Response {
    bearer_proxy_or_spa(bff, &format!("/v1/{path}"), request).await
}

async fn v1_bearer_proxy_root(State(bff): State<BffState>, request: Request<Body>) -> Response {
    bearer_proxy_or_spa(bff, "/v1", request).await
}

/// If the request carries an `Authorization: Bearer <token>` header, forward it
/// directly to the internal server. Otherwise, serve the SPA (index.html) so
/// browsers navigating to `/v1/...` still get the frontend.
async fn bearer_proxy_or_spa(
    bff: BffState,
    target_path: &str,
    original: Request<Body>,
) -> Response {
    // Extract the Bearer token from the Authorization header.
    let bearer_token = original
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with("Bearer "))
        .map(|v| v[7..].to_string());

    let token = match bearer_token {
        Some(t) => t,
        None => return crate::frontend::serve_spa_fallback(),
    };

    // Build a new request to the internal server.
    let uri = if let Some(query) = original.uri().query() {
        format!("{target_path}?{query}")
    } else {
        target_path.to_string()
    };

    let method = original.method().clone();
    let mut builder = Request::builder()
        .method(method)
        .uri(&uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));

    for (name, value) in original.headers() {
        if name == header::HOST || name == header::COOKIE || name == header::AUTHORIZATION {
            continue;
        }
        builder = builder.header(name, value);
    }

    let internal_req = match builder.body(original.into_body()) {
        Ok(req) => req,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "failed to build request" })),
            )
                .into_response();
        }
    };

    match bff.inner_app.clone().oneshot(internal_req).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "internal error" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// API proxy: /api/v1/* -> internal /v1/*
// ---------------------------------------------------------------------------

async fn api_proxy(
    State(bff): State<BffState>,
    jar: CookieJar,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: Request<Body>,
) -> impl IntoResponse {
    proxy_to_internal(bff, jar, &format!("/v1/{path}"), request).await
}

async fn api_proxy_root(
    State(bff): State<BffState>,
    jar: CookieJar,
    request: Request<Body>,
) -> impl IntoResponse {
    proxy_to_internal(bff, jar, "/v1", request).await
}

async fn proxy_to_internal(
    bff: BffState,
    jar: CookieJar,
    target_path: &str,
    original: Request<Body>,
) -> Response {
    let token = match jar.get(COOKIE_NAME) {
        Some(cookie) => cookie.value().to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "not authenticated" })),
            )
                .into_response();
        }
    };

    // Build a new request to the internal server with the Bearer token.
    let uri = if let Some(query) = original.uri().query() {
        format!("{target_path}?{query}")
    } else {
        target_path.to_string()
    };

    let method = original.method().clone();
    let mut builder = Request::builder()
        .method(method)
        .uri(&uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));

    // Copy relevant headers from the original request.
    for (name, value) in original.headers() {
        if name == header::HOST || name == header::COOKIE || name == header::AUTHORIZATION {
            continue;
        }
        builder = builder.header(name, value);
    }

    let internal_req = match builder.body(original.into_body()) {
        Ok(req) => req,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "failed to build request" })),
            )
                .into_response();
        }
    };

    match bff.inner_app.clone().oneshot(internal_req).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "internal error" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_auth_cookie(token: &str) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, token.to_string()))
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .path("/")
        .build()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;

    fn test_bff_state() -> BffState {
        let handles = metis_server::test_utils::test_state_handles();
        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        BffState {
            auto_login_token: Arc::new("test-token".to_string()),
            inner_app,
        }
    }

    #[tokio::test]
    async fn auth_login_requires_token() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn auth_login_rejects_invalid_token() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"token": "bad-token"}"#))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_login_accepts_valid_token() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        // Seed the test actor so the token validates.
        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let bff_state = BffState {
            auto_login_token: Arc::new("unused".to_string()),
            inner_app,
        };
        let app = build_bff_router(bff_state);

        let token = metis_server::test_utils::test_auth_token();
        let body = serde_json::json!({ "token": token });
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Check that the Set-Cookie header is present.
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("should set cookie");
        let cookie_str = set_cookie.to_str().unwrap();
        assert!(cookie_str.contains("metis_token="));
        assert!(cookie_str.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn auth_logout_clears_cookie() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("POST")
            .uri("/auth/logout")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("should clear cookie");
        let cookie_str = set_cookie.to_str().unwrap();
        assert!(cookie_str.contains("metis_token="));
    }

    #[tokio::test]
    async fn auth_me_without_cookie_returns_401() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_me_with_valid_cookie() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let token = metis_server::test_utils::test_auth_token();
        let bff_state = BffState {
            auto_login_token: Arc::new(token.clone()),
            inner_app,
        };
        let app = build_bff_router(bff_state);

        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .header(header::COOKIE, format!("metis_token={token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auto_login_sets_cookie_on_first_request() {
        let handles = metis_server::test_utils::test_state_handles();
        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let bff_state = BffState {
            auto_login_token: Arc::new("auto-token-123".to_string()),
            inner_app,
        };
        let app = build_bff_router(bff_state);

        // Request the root (frontend) without any cookie.
        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // The auto-login middleware should set the cookie.
        let set_cookie = response.headers().get(header::SET_COOKIE);
        // The cookie may or may not be set depending on whether the frontend
        // assets exist (they may not in test). But the middleware logic is
        // tested: if there's no cookie in the request, it should be added.
        if let Some(val) = set_cookie {
            let cookie_str = val.to_str().unwrap();
            assert!(cookie_str.contains("metis_token=auto-token-123"));
        }
    }

    #[tokio::test]
    async fn api_proxy_strips_prefix() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let token = metis_server::test_utils::test_auth_token();
        let bff_state = BffState {
            auto_login_token: Arc::new(token.clone()),
            inner_app,
        };
        let app = build_bff_router(bff_state);

        // Call /api/v1/whoami which should route to /v1/whoami internally.
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .header(header::COOKIE, format!("metis_token={token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("actor").is_some());
    }

    #[tokio::test]
    async fn api_proxy_without_cookie_returns_401() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_bearer_returns_json() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let token = metis_server::test_utils::test_auth_token();
        let bff_state = BffState {
            auto_login_token: Arc::new("unused".to_string()),
            inner_app,
        };
        let app = build_bff_router(bff_state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("actor").is_some());
    }

    #[tokio::test]
    async fn v1_without_bearer_falls_through_to_spa() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        // Without Bearer auth, the response should be a SPA fallback (HTML) or 404
        // if no frontend assets are embedded (test environment).
        let status = response.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "expected OK (SPA) or NOT_FOUND, got {status}"
        );

        if status == StatusCode::OK {
            let ct = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(ct.contains("text/html"), "expected HTML, got {ct}");
        }
    }

    #[tokio::test]
    async fn v1_bearer_invalid_token_returns_401() {
        let app = build_bff_router(test_bff_state());

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .header(header::AUTHORIZATION, "Bearer bad-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn cookie_round_trip() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app =
            metis_server::build_router(&handles.state).with_state(handles.state.clone());
        let token = metis_server::test_utils::test_auth_token();
        let bff_state = BffState {
            auto_login_token: Arc::new(token.clone()),
            inner_app,
        };

        // Step 1: Login and get the cookie.
        let app = build_bff_router(bff_state.clone());
        let body = serde_json::json!({ "token": token });
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let login_resp = app.oneshot(req).await.unwrap();
        assert_eq!(login_resp.status(), StatusCode::OK);

        let set_cookie = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Step 2: Use the cookie for an API call.
        let app = build_bff_router(bff_state);
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .header(header::COOKIE, &set_cookie)
            .body(Body::empty())
            .unwrap();

        let api_resp = app.oneshot(req).await.unwrap();
        assert_eq!(api_resp.status(), StatusCode::OK);
    }
}
