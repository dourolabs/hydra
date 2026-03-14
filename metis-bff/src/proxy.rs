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

use crate::auth::COOKIE_NAME;
use crate::state::BffState;
use crate::upstream::Upstream;

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
    forward_to_upstream(&bff, &format!("/v1/{path}"), Some(token), request).await
}

async fn api_proxy_root<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
    request: Request<Body>,
) -> impl IntoResponse {
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
    let mut builder = Request::builder().method(method).uri(&uri);

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
        Ok(response) => response,
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "internal error" })),
        )
            .into_response(),
    }
}
