use axum::Json;
use hydra_common::api::v1::version::VersionResponse;
use tracing::info;

pub async fn get_version() -> Json<VersionResponse> {
    info!("get_version invoked");
    let response = VersionResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    info!(version = %response.version, "get_version completed");
    Json(response)
}
