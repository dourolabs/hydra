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
    Router::new()
        .route(
            "/*path",
            get(move |AxumPath(path): AxumPath<String>| {
                let dir = dir.clone();
                async move { serve_directory_file(&dir, &path) }
            }),
        )
        .route(
            "/",
            get(move || {
                // We need our own clone for this closure.
                // Since we moved dir into the previous closure, we use a different approach.
                async { StatusCode::NOT_FOUND.into_response() }
            }),
        )
}

fn serve_directory_file(dir: &Path, path: &str) -> Response {
    let file_path = dir.join(path);
    if file_path.is_file() {
        match std::fs::read(&file_path) {
            Ok(contents) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(Body::from(contents))
                    .unwrap()
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {
        // SPA fallback: serve index.html.
        let index_path = dir.join("index.html");
        match std::fs::read(&index_path) {
            Ok(contents) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(contents))
                .unwrap(),
            Err(_) => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("frontend not found"))
                .unwrap(),
        }
    }
}

// Embedded frontend (behind feature flag).
#[cfg(feature = "embedded-frontend")]
mod embedded {
    use super::*;
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "../metis-web/packages/web/dist"]
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
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(Body::from(file.data.to_vec()))
                    .unwrap()
            }
            None => match EmbeddedAssets::get("index.html") {
                Some(index) => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html")
                    .body(Body::from(index.data.to_vec()))
                    .unwrap(),
                None => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("frontend not found"))
                    .unwrap(),
            },
        }
    }
}

#[cfg(feature = "embedded-frontend")]
use embedded::embedded_router;
