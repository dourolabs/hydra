use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{header, Request},
    middleware::{self, Next},
    response::Response,
    Router,
};
use axum_extra::extract::cookie::CookieJar;
use metis_bff::{
    auth::build_auth_cookie, auth::COOKIE_NAME, BffConfig, BffState, FrontendAssets,
    InProcessUpstream,
};

/// Auto-login token for single-player mode.
#[derive(Clone)]
pub struct AutoLoginState {
    pub token: Arc<String>,
}

/// Build the complete BFF + server + frontend router for single-player mode.
///
/// This wraps the `metis-bff` library router with the auto-login middleware
/// that is specific to single-player mode.
pub fn build_bff_router(inner_app: Router, auto_login_token: String) -> Router {
    let upstream = InProcessUpstream::new(inner_app);
    let config = BffConfig {
        auth_login_enabled: true,
        cookie_secure: false,
        frontend_assets: FrontendAssets::Embedded,
        cache: None,
    };
    let bff_state = BffState::new(upstream, config);
    let bff_router = metis_bff::build_bff_router(bff_state);

    let auto_login = AutoLoginState {
        token: Arc::new(auto_login_token),
    };

    bff_router.layer(middleware::from_fn_with_state(
        auto_login,
        auto_login_middleware,
    ))
}

/// Middleware that ensures the `metis_token` cookie is always set.
/// For single-player mode, the user is always logged in.
async fn auto_login_middleware(
    State(auto_login): State<AutoLoginState>,
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
    let cookie = build_auth_cookie(auto_login.token.as_str(), false);
    let (mut parts, body) = response.into_parts();
    if let Ok(value) = cookie.to_string().parse() {
        parts.headers.append(header::SET_COOKIE, value);
    }
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;

    fn test_bff_app() -> Router {
        let handles = metis_server::test_utils::test_state_handles();
        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        build_bff_router(inner_app, "test-token".to_string())
    }

    #[tokio::test]
    async fn auth_login_requires_token() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn auth_login_rejects_invalid_token() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"token": "bad-token"}"#))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_login_accepts_valid_token() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let token = metis_server::test_utils::test_auth_token();
        let app = build_bff_router(inner_app, "unused".to_string());

        let body = serde_json::json!({ "token": token });
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

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
        let app = test_bff_app();

        let req = Request::builder()
            .method("POST")
            .uri("/auth/logout")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("should clear cookie");
        let cookie_str = set_cookie.to_str().unwrap();
        assert!(cookie_str.contains("metis_token="));
    }

    #[tokio::test]
    async fn auth_me_without_cookie_returns_401() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
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
        let app = build_bff_router(inner_app, token.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .header(header::COOKIE, format!("metis_token={token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);
    }

    #[tokio::test]
    async fn auto_login_sets_cookie_on_first_request() {
        let handles = metis_server::test_utils::test_state_handles();
        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let app = build_bff_router(inner_app, "auto-token-123".to_string());

        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("auto-login middleware must set metis_token cookie on first request");
        let cookie_str = set_cookie.to_str().unwrap();
        assert!(
            cookie_str.contains("metis_token=auto-token-123"),
            "expected metis_token=auto-token-123, got {cookie_str}"
        );
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
        let app = build_bff_router(inner_app, token.clone());

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .header(header::COOKIE, format!("metis_token={token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("actor").is_some());
    }

    #[tokio::test]
    async fn api_proxy_without_cookie_returns_401() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
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
        let app = build_bff_router(inner_app, "unused".to_string());

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("actor").is_some());
    }

    #[tokio::test]
    async fn v1_without_bearer_returns_401() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_bearer_invalid_token_returns_401() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("GET")
            .uri("/v1/whoami")
            .header(header::AUTHORIZATION, "Bearer bad-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_returns_ok_json() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("should have content-type")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.contains("application/json"),
            "expected application/json, got {content_type}"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body, serde_json::json!({"status": "ok"}));
    }

    #[tokio::test]
    async fn auto_login_end_to_end() {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app =
            metis_server::build_router(&handles.state).with_state(handles.state.clone());
        let token = metis_server::test_utils::test_auth_token();

        // Step 1: Request GET / without any cookie.
        let app = build_bff_router(inner_app.clone(), token.clone());
        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("auto-login middleware must set cookie on GET /");
        let set_cookie_str = set_cookie.to_str().unwrap();
        assert!(
            set_cookie_str.contains("metis_token="),
            "expected metis_token cookie, got {set_cookie_str}"
        );

        let cookie_header = set_cookie_str
            .split(';')
            .next()
            .expect("cookie should have a value part");

        // Step 2: Request GET /auth/me with the cookie from step 1.
        let app = build_bff_router(inner_app, token);
        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .header(header::COOKIE, cookie_header)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "GET /auth/me with auto-login cookie should return 200 OK"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            body.get("actor").is_some(),
            "response should contain actor info"
        );
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

        // Step 1: Login and get the cookie.
        let app = build_bff_router(inner_app.clone(), token.clone());
        let body = serde_json::json!({ "token": token });
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let login_resp = app.oneshot(req).await.unwrap();
        assert_eq!(login_resp.status(), http::StatusCode::OK);

        let set_cookie = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Step 2: Use the cookie for an API call.
        let app = build_bff_router(inner_app, token);
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .header(header::COOKIE, &set_cookie)
            .body(Body::empty())
            .unwrap();

        let api_resp = app.oneshot(req).await.unwrap();
        assert_eq!(api_resp.status(), http::StatusCode::OK);
    }
}
