use std::path::Path;

use axum::{
    body::Body,
    extract::Path as AxumPath,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::config::FrontendAssets;

/// Build a frontend-serving router based on the asset mode.
pub fn router(assets: &FrontendAssets) -> Option<Router> {
    match assets {
        FrontendAssets::Embedded => {
            #[cfg(feature = "embedded-frontend")]
            {
                Some(embedded_router())
            }
            #[cfg(not(feature = "embedded-frontend"))]
            {
                tracing::warn!(
                    "FrontendAssets::Embedded requested but embedded-frontend feature not enabled"
                );
                None
            }
        }
        FrontendAssets::Directory(path) => Some(directory_router(path.clone())),
        FrontendAssets::None => None,
    }
}

/// Router that serves assets from a filesystem directory with SPA fallback.
fn directory_router(dir: std::path::PathBuf) -> Router {
    let dir_for_root = dir.clone();
    Router::new()
        .route(
            "/*path",
            get(move |AxumPath(path): AxumPath<String>| {
                let dir = dir.clone();
                async move { serve_directory_file(&dir, &path).await }
            }),
        )
        .route(
            "/",
            get(move || async move {
                serve_directory_file(&dir_for_root, "index.html")
                    .await
                    .into_response()
            }),
        )
}

fn cache_control_for(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

async fn serve_directory_file(dir: &Path, path: &str) -> Response {
    let file_path = dir.join(path);
    let is_file = tokio::fs::metadata(&file_path)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false);
    if is_file {
        match tokio::fs::read(&file_path).await {
            Ok(contents) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                let cache_control = cache_control_for(path);
                tracing::info!(
                    path,
                    status = 200u16,
                    cache_control,
                    source = "directory",
                    "serving frontend asset"
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .header(header::CACHE_CONTROL, cache_control)
                    .body(Body::from(contents))
                    .unwrap()
            }
            Err(_) => {
                tracing::info!(
                    path,
                    status = 500u16,
                    source = "directory",
                    "failed to read frontend asset"
                );
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    } else {
        // SPA fallback: serve index.html.
        tracing::info!(
            path,
            source = "directory",
            "asset not found, falling back to index.html"
        );
        let index_path = dir.join("index.html");
        match tokio::fs::read(&index_path).await {
            Ok(contents) => {
                tracing::info!(
                    path,
                    status = 200u16,
                    cache_control = "no-cache",
                    source = "directory",
                    "serving SPA fallback"
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body(Body::from(contents))
                    .unwrap()
            }
            Err(_) => {
                tracing::info!(
                    path,
                    status = 404u16,
                    source = "directory",
                    "frontend not found"
                );
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("frontend not found"))
                    .unwrap()
            }
        }
    }
}

// Embedded frontend (behind feature flag).
#[cfg(feature = "embedded-frontend")]
mod embedded {
    use super::*;
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "../hydra-web/packages/web/dist"]
    struct EmbeddedAssets;

    pub fn embedded_router() -> Router {
        Router::new()
            .route("/*path", get(serve_embedded_asset))
            .route("/", get(serve_embedded_index))
    }

    async fn serve_embedded_index() -> impl IntoResponse {
        serve_embedded_file("index.html")
    }

    async fn serve_embedded_asset(AxumPath(path): AxumPath<String>) -> impl IntoResponse {
        serve_embedded_file(&path)
    }

    fn serve_embedded_file(path: &str) -> Response {
        match EmbeddedAssets::get(path) {
            Some(file) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                let cache_control = cache_control_for(path);
                tracing::info!(
                    path,
                    status = 200u16,
                    cache_control,
                    source = "embedded",
                    "serving frontend asset"
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .header(header::CACHE_CONTROL, cache_control)
                    .body(Body::from(file.data.to_vec()))
                    .unwrap()
            }
            None => {
                tracing::info!(
                    path,
                    source = "embedded",
                    "asset not found, falling back to index.html"
                );
                match EmbeddedAssets::get("index.html") {
                    Some(index) => {
                        tracing::info!(
                            path,
                            status = 200u16,
                            cache_control = "no-cache",
                            source = "embedded",
                            "serving SPA fallback"
                        );
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "text/html")
                            .header(header::CACHE_CONTROL, "no-cache")
                            .body(Body::from(index.data.to_vec()))
                            .unwrap()
                    }
                    None => {
                        tracing::info!(
                            path,
                            status = 404u16,
                            source = "embedded",
                            "frontend not found"
                        );
                        Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Body::from("frontend not found"))
                            .unwrap()
                    }
                }
            }
        }
    }
}

#[cfg(feature = "embedded-frontend")]
use embedded::embedded_router;
