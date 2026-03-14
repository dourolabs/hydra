use std::future::Future;
use std::pin::Pin;

use axum::{body::Body, response::Response};
use http::Request;
use tower::ServiceExt;

/// Error type for upstream forwarding failures.
#[derive(Debug)]
pub struct UpstreamError(pub String);

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream error: {}", self.0)
    }
}

impl std::error::Error for UpstreamError {}

/// Abstraction over how the BFF reaches the upstream metis-server.
pub trait Upstream: Send + Sync + 'static {
    /// Forward a request to the upstream metis-server and return the response.
    fn forward(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>>;
}

/// In-process upstream that calls the inner Axum router via `oneshot()`.
/// Used by `metis-single-player` for zero network overhead.
#[derive(Clone)]
pub struct InProcessUpstream {
    router: axum::Router,
}

impl InProcessUpstream {
    pub fn new(router: axum::Router) -> Self {
        Self { router }
    }
}

impl Upstream for InProcessUpstream {
    fn forward(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
        let router = self.router.clone();
        Box::pin(async move {
            router
                .oneshot(request)
                .await
                .map_err(|e| UpstreamError(e.to_string()))
        })
    }
}
