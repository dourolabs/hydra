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

/// Abstraction over how the BFF reaches the upstream hydra-server.
pub trait Upstream: Send + Sync + 'static {
    /// Forward a request to the upstream hydra-server and return the response.
    fn forward(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>>;

    /// Forward a request intended for long-lived streaming (e.g. SSE).
    /// Uses a client without a request timeout so the connection stays open indefinitely.
    /// The default implementation delegates to `forward()`.
    fn forward_streaming(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
        self.forward(request)
    }
}

/// In-process upstream that calls the inner Axum router via `oneshot()`.
/// Used by `hydra-single-player` for zero network overhead.
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

/// HTTP upstream that forwards requests to a remote hydra-server via reqwest.
/// Used by the standalone `hydra-bff-server` binary in multi-player deployments.
#[derive(Clone)]
pub struct HttpUpstream {
    client: reqwest::Client,
    /// Client without a request timeout, used for SSE / long-lived streaming connections.
    streaming_client: reqwest::Client,
    base_url: String,
}

impl HttpUpstream {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(60))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("failed to build reqwest client");
        let streaming_client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(5))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("failed to build streaming reqwest client");
        Self {
            client,
            streaming_client,
            base_url,
        }
    }
}

impl HttpUpstream {
    /// Shared forwarding logic used by both `forward` and `forward_streaming`.
    fn forward_with_client(
        client: reqwest::Client,
        base_url: String,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
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

impl Upstream for HttpUpstream {
    fn forward(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
        Self::forward_with_client(self.client.clone(), self.base_url.clone(), request)
    }

    fn forward_streaming(
        &self,
        request: Request<Body>,
    ) -> Pin<Box<dyn Future<Output = Result<Response, UpstreamError>> + Send>> {
        Self::forward_with_client(
            self.streaming_client.clone(),
            self.base_url.clone(),
            request,
        )
    }
}
