use axum::Router;
use metis_bff::{BffConfig, BffState, FrontendAssets, InProcessUpstream};

/// Build the complete BFF + server + frontend router for single-player mode.
///
/// Sets `auto_login_token` on BffState so that all proxied requests are
/// automatically authenticated server-side without cookies.
pub fn build_bff_router(inner_app: Router, auto_login_token: String) -> Router {
    let upstream = InProcessUpstream::new(inner_app);
    let config = BffConfig {
        auth_login_enabled: false,
        cookie_secure: false,
        frontend_assets: FrontendAssets::Embedded,
        cache: None,
    };
    let bff_state = BffState::new(upstream, config).with_auto_login_token(auto_login_token);
    metis_bff::build_bff_router(bff_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header;
    use http::Request;
    use tower::ServiceExt;

    fn test_bff_app() -> Router {
        let handles = metis_server::test_utils::test_state_handles();
        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        build_bff_router(inner_app, "test-token".to_string())
    }

    async fn test_bff_app_with_actor() -> (Router, String) {
        let handles = metis_server::test_utils::test_state_handles();
        let store = handles.store.clone();

        let actor = metis_server::test_utils::test_actor();
        let system_ref = metis_server::domain::actors::ActorRef::test();
        let _ = store.add_actor(actor, &system_ref).await;

        let inner_app = metis_server::build_router(&handles.state).with_state(handles.state);
        let token = metis_server::test_utils::test_auth_token();
        let app = build_bff_router(inner_app, token.clone());
        (app, token)
    }

    #[tokio::test]
    async fn auth_login_is_noop_in_single_player() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_logout_returns_404_in_single_player() {
        let app = test_bff_app();

        let req = Request::builder()
            .method("POST")
            .uri("/auth/logout")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn auth_me_without_cookie_returns_user_in_single_player() {
        let (app, _token) = test_bff_app_with_actor().await;

        let req = Request::builder()
            .method("GET")
            .uri("/auth/me")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "GET /auth/me should succeed without cookies in single-player mode"
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
    async fn api_proxy_without_cookie_succeeds_in_single_player() {
        let (app, _token) = test_bff_app_with_actor().await;

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "GET /api/v1/whoami should succeed without cookies in single-player mode"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("actor").is_some());
    }

    #[tokio::test]
    async fn v1_bearer_returns_json() {
        let (app, token) = test_bff_app_with_actor().await;

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
}
