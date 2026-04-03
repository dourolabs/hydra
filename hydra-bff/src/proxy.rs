use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use axum_extra::extract::cookie::CookieJar;
use http::Request;

use crate::state::BffState;
use crate::upstream::Upstream;

/// Paths under `/api/v1/` that are public (do not require authentication).
/// When no cookie is present, these are forwarded without a Bearer token.
const PUBLIC_API_PATHS: &[&str] = &["github/app/client-id"];

/// Build the `/api/v1` sub-router (cookie-to-Bearer translation).
pub fn api_router<U: Upstream>() -> Router<BffState<U>> {
    Router::new()
        .route("/*path", any(api_proxy::<U>))
        .route("/", any(api_proxy_root::<U>))
}

/// Build the `/v1` sub-router (direct pass-through).
pub fn v1_router<U: Upstream>() -> Router<BffState<U>> {
    Router::new()
        .route("/*path", any(v1_pass_through::<U>))
        .route("/", any(v1_pass_through_root::<U>))
}

async fn api_proxy<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: Request<Body>,
) -> impl IntoResponse {
    let is_public = PUBLIC_API_PATHS.iter().any(|p| *p == path);
    let token = match bff.resolve_token(&jar) {
        Some(t) => Some(t),
        None if is_public => None,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "not authenticated" })),
            )
                .into_response();
        }
    };
    forward_to_upstream(&bff, &format!("/v1/{path}"), token, request).await
}

async fn api_proxy_root<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
    request: Request<Body>,
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
    forward_to_upstream(&bff, "/v1", Some(token), request).await
}

async fn v1_pass_through<U: Upstream>(
    State(bff): State<BffState<U>>,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: Request<Body>,
) -> Response {
    forward_to_upstream(&bff, &format!("/v1/{path}"), None, request).await
}

async fn v1_pass_through_root<U: Upstream>(
    State(bff): State<BffState<U>>,
    request: Request<Body>,
) -> Response {
    forward_to_upstream(&bff, "/v1", None, request).await
}

/// Forward a request to the upstream at `target_path`.
///
/// If `override_token` is `Some`, the Authorization header is set to
/// `Bearer <token>` (cookie-to-Bearer translation for `/api/v1/*`).
/// If `None`, the original Authorization header is passed through
/// (transparent mirror for `/v1/*`).
pub(crate) async fn forward_to_upstream<U: Upstream>(
    bff: &BffState<U>,
    target_path: &str,
    override_token: Option<String>,
    original: Request<Body>,
) -> Response {
    let uri = if let Some(query) = original.uri().query() {
        format!("{target_path}?{query}")
    } else {
        target_path.to_string()
    };

    let method = original.method().clone();
    tracing::info!(method = %method, upstream_path = %uri, "proxying request to upstream");
    let mut builder = Request::builder().method(&method).uri(&uri);

    // Set the Authorization header: either override with token or pass through.
    if let Some(token) = override_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    for (name, value) in original.headers() {
        if name == header::HOST || name == header::COOKIE {
            continue;
        }
        // Skip Authorization if we're overriding it.
        if name == header::AUTHORIZATION
            && builder
                .headers_ref()
                .is_some_and(|h| h.contains_key(header::AUTHORIZATION))
        {
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

    match bff.upstream.forward(internal_req).await {
        Ok(response) => {
            tracing::info!(method = %method, upstream_path = %uri, status = %response.status(), "upstream response received");
            response
        }
        Err(e) => {
            tracing::error!(method = %method, upstream_path = %uri, error = %e, "upstream request failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "internal error" })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BffConfig, BffState, FrontendAssets, InProcessUpstream};
    use tower::ServiceExt;

    /// Build a multi-player BFF (no auto_login_token) backed by an in-process
    /// hydra-server so we can test cookie-to-Bearer translation and public routes.
    fn test_bff_multiplayer() -> Router {
        let handles = hydra_server::test_utils::test_state_handles();
        let inner_app = hydra_server::build_router(&handles.state).with_state(handles.state);
        let upstream = InProcessUpstream::new(inner_app);
        let config = BffConfig {
            cookie_secure: false,
            frontend_assets: FrontendAssets::None,
        };
        let bff_state = BffState::new(upstream, config, None);
        crate::build_bff_router(bff_state)
    }

    #[tokio::test]
    async fn github_client_id_accessible_without_cookie() {
        let app = test_bff_multiplayer();

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/github/app/client-id")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET /api/v1/github/app/client-id should succeed without a cookie"
        );

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            body.get("client_id").is_some(),
            "response should contain client_id"
        );
    }

    #[tokio::test]
    async fn other_api_routes_still_require_cookie() {
        let app = test_bff_multiplayer();

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/whoami")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "GET /api/v1/whoami should return 401 without a cookie"
        );
    }
}
