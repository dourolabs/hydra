use axum::Json;
use hydra_common::api::v1::version::VersionResponse;

pub async fn get_version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
