use std::time::Duration;

use axum::{body::Body, extract::State, http::Request, response::Response, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::proxy;
use crate::state::BffState;
use crate::upstream::Upstream;
use crate::{auth, frontend, sse};

/// Build the complete BFF router with all routes.
///
/// The returned router handles:
/// - `/auth/*` -- login, logout, me (cookie-based auth)
/// - `/api/v1/events` -- SSE relay with cookie-to-Bearer translation
/// - `/api/v1/*` -- API proxy with cookie-to-Bearer translation
/// - `/v1/*` -- direct pass-through (preserves existing Authorization header)
/// - `/health` -- health check proxy to upstream
/// - `/*` -- frontend SPA fallback (if configured)
pub fn build_bff_router<U: Upstream>(state: BffState<U>) -> Router {
    let auth_routes = auth::router::<U>();
    let api_routes = proxy::api_router::<U>();
    let v1_routes = proxy::v1_router::<U>();

    let mut router = Router::new()
        .nest("/auth", auth_routes)
        // SSE events endpoint must be registered before the wildcard /api/v1/*
        .route(
            "/api/v1/events",
            get(sse::sse_relay::<U>).post(sse::sse_relay::<U>),
        )
        .nest("/api/v1", api_routes)
        .nest("/v1", v1_routes)
        .route("/health", get(health_proxy::<U>));

    // Add frontend serving as fallback if configured.
    if let Some(frontend_router) = frontend::router(&state.config.frontend_assets) {
        router = router.fallback_service(frontend_router);
    }

    let trace_layer = TraceLayer::new_for_http()
        .on_request(|request: &Request<Body>, _span: &tracing::Span| {
            tracing::debug!(
                method = %request.method(),
                path = %request.uri().path(),
                "started processing request"
            );
        })
        .on_response(
            |response: &Response, latency: Duration, _span: &tracing::Span| {
                let status = response.status().as_u16();
                let latency_ms = latency.as_millis();

                if status >= 500 {
                    tracing::error!(status, latency_ms, "request failed with server error");
                } else if latency > Duration::from_secs(1) {
                    tracing::warn!(status, latency_ms, "slow request");
                } else {
                    tracing::info!(status, latency_ms, "request completed");
                }
            },
        );

    router.layer(trace_layer).with_state(state)
}

async fn health_proxy<U: Upstream>(
    State(bff): State<BffState<U>>,
    request: Request<Body>,
) -> Response {
    proxy::forward_to_upstream(&bff, "/health", None, request).await
}
