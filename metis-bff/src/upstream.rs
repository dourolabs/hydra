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

/// HTTP upstream that forwards requests to a remote metis-server via reqwest.
/// Used by the standalone `metis-bff-server` binary in multi-player deployments.
#[derive(Clone)]
pub struct HttpUpstream {
    client: reqwest::Client,
    base_url: String,
}

impl HttpUpstream {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build reqwest client");
        Self { client, base_url }
    }
}

impl Upstream for HttpUpstream {
    fn forward(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        Box::pin(async move {
            let (parts, body) = request.into_parts();

            let url = format!(
                "{}{}",
                base_url.trim_end_matches('/'),
                parts
                    .uri
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/")
            );

            let body_bytes = axum::body::to_bytes(body, 64 * 1024 * 1024)
                .await
                .map_err(|e| UpstreamError(format!("failed to read request body: {e}")))?;

            let mut req_builder = client.request(parts.method, &url);

            for (name, value) in &parts.headers {
                req_builder = req_builder.header(name, value);
            }

            let reqwest_resp = req_builder
                .body(body_bytes)
                .send()
                .await
                .map_err(|e| UpstreamError(format!("upstream request failed: {e}")))?;

            let status = reqwest_resp.status();
            let headers = reqwest_resp.headers().clone();

            // Stream the response body to support SSE without buffering.
            let byte_stream = reqwest_resp.bytes_stream();
            let body = Body::from_stream(byte_stream);

            let mut response = Response::builder().status(status);
            for (name, value) in &headers {
                response = response.header(name, value);
            }

            response
                .body(body)
                .map_err(|e| UpstreamError(format!("failed to build response: {e}")))
        })
    }
}
