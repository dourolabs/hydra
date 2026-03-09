use axum::{
    Router,
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

use crate::app::AppState;

#[derive(Embed)]
#[folder = "../metis-web/packages/web/dist/"]
struct FrontendAssets;

/// Build an Axum router that serves the embedded frontend assets.
///
/// - Known static files (JS, CSS, images) are served at their path with the
///   correct Content-Type.
/// - All other GET requests fall back to `index.html` for SPA client-side
///   routing.
pub fn router() -> Router<AppState> {
    Router::new().fallback(serve_frontend)
}

async fn serve_frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the exact file first.
    if !path.is_empty() {
        if let Some(file) = FrontendAssets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                file.data,
            )
                .into_response();
        }
    }

    // SPA fallback: serve index.html for any unmatched route.
    match FrontendAssets::get("index.html") {
        Some(file) => Html(file.data).into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}
