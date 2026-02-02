pub mod config;
pub mod s3;

use anyhow::Result;
use axum::{Json, Router, extract::DefaultBodyLimit, routing::get};
use serde_json::json;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Maximum body size for PUT requests (150 MB).
pub const MAX_BODY_SIZE: usize = 150 * 1024 * 1024;

pub fn build_router(storage_root: PathBuf) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .merge(s3::router(storage_root))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(listener: TcpListener, storage_root: PathBuf) -> Result<()> {
    axum::serve(listener, build_router(storage_root)).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    info!("healthz invoked");
    Json(json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    /// Build a router with a custom body size limit for testing.
    fn build_router_with_limit(storage_root: PathBuf, limit: usize) -> Router {
        Router::new()
            .route("/healthz", get(healthz))
            .merge(s3::router(storage_root))
            .layer(DefaultBodyLimit::max(limit))
            .layer(TraceLayer::new_for_http())
    }

    #[test]
    fn max_body_size_is_150mb() {
        assert_eq!(MAX_BODY_SIZE, 150 * 1024 * 1024);
    }

    #[tokio::test]
    async fn body_limit_rejects_oversized_requests() {
        let dir = tempdir().expect("temp dir");
        // Use a small limit (1KB) for testing
        let limit = 1024;
        let router = build_router_with_limit(dir.path().to_path_buf(), limit);

        // Create a body that exceeds the limit
        let oversized_body = vec![b'x'; limit + 1];
        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test-key")
                    .body(Body::from(oversized_body))
                    .unwrap(),
            )
            .await
            .expect("response");

        // Axum returns 413 Payload Too Large when body limit is exceeded
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn body_limit_allows_requests_within_limit() {
        let dir = tempdir().expect("temp dir");
        // Use a small limit (1KB) for testing
        let limit = 1024;
        let router = build_router_with_limit(dir.path().to_path_buf(), limit);

        // Create a body that is exactly at the limit
        let valid_body = vec![b'x'; limit];
        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test-key")
                    .body(Body::from(valid_body))
                    .unwrap(),
            )
            .await
            .expect("response");

        // Request should succeed
        assert_eq!(response.status(), StatusCode::OK);
    }
}
