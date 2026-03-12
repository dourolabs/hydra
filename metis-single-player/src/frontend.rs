use axum::{
    body::Body,
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../metis-web/packages/web/dist"]
struct FrontendAssets;

/// Build an Axum router that serves the embedded frontend assets.
///
/// Static files are served at their exact path. Any path that does not match a
/// static asset falls back to `index.html` (SPA routing).
pub fn router() -> Router {
    Router::new()
        .route("/*path", get(serve_asset))
        .route("/", get(serve_index))
}

async fn serve_index() -> impl IntoResponse {
    serve_file("index.html")
}

async fn serve_asset(Path(path): Path<String>) -> impl IntoResponse {
    serve_file(&path)
}


fn serve_file(path: &str) -> Response {
    match FrontendAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.to_vec()))
                .unwrap()
        }
        // SPA fallback: serve index.html for any unmatched path.
        None => match FrontendAssets::get("index.html") {
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
