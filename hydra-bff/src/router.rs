use axum::{body::Body, extract::State, http::Request, response::Response, routing::get, Router};

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

    router.with_state(state)
}

async fn health_proxy<U: Upstream>(
    State(bff): State<BffState<U>>,
    request: Request<Body>,
) -> Response {
    proxy::forward_to_upstream(&bff, "/health", None, request).await
}
