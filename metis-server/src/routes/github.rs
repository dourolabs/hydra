use crate::{app::AppState, routes::jobs::ApiError};
use axum::{Json, extract::State};
use metis_common::github::GithubAppClientIdResponse;
use tracing::info;

pub async fn get_github_app_client_id(
    State(state): State<AppState>,
) -> Result<Json<GithubAppClientIdResponse>, ApiError> {
    info!("get_github_app_client_id invoked");
    let client_id = state
        .config
        .github_app
        .client_id()
        .ok_or_else(|| ApiError::not_found("github app client_id not configured"))?;

    Ok(Json(GithubAppClientIdResponse { client_id }))
}
