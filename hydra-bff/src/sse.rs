use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use http::Request;

use crate::state::BffState;
use crate::upstream::Upstream;

/// SSE relay handler for `/api/v1/events`.
///
/// Extracts the auth token (from auto_login_token or cookie), translates it
/// to a Bearer token, forwards to the upstream `/v1/events` endpoint, and
/// streams the SSE response back with proper headers.
pub async fn sse_relay<U: Upstream>(
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

    let mut uri = "/v1/events".to_string();
    if let Some(query) = request.uri().query() {
        uri = format!("{uri}?{query}");
    }

    let mut builder = Request::builder()
        .method(request.method())
        .uri(&uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));

    // Forward Last-Event-ID if present.
    if let Some(last_event_id) = request.headers().get("Last-Event-ID") {
        builder = builder.header("Last-Event-ID", last_event_id);
    }

    let upstream_req = match builder.body(Body::empty()) {
        Ok(req) => req,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "failed to build request" })),
            )
                .into_response();
        }
    };

    match bff.upstream.forward(upstream_req).await {
        Ok(upstream_resp) => {
            // Stream the SSE response back with proper headers.
            let (parts, body) = upstream_resp.into_parts();
            let mut response = Response::from_parts(parts, body);

            // Ensure SSE content type and streaming headers.
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
            response
                .headers_mut()
                .insert(header::CONNECTION, "keep-alive".parse().unwrap());

            response.into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "internal error" })),
        )
            .into_response(),
    }
}
