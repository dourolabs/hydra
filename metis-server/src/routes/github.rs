use crate::{app::AppState, domain::actors::Actor, routes::jobs::ApiError};
use axum::{Extension, Json, extract::State};
use metis_common::github::{GithubAppClientIdResponse, GithubTokenResponse};
use tracing::info;

pub async fn get_github_app_client_id(
    State(state): State<AppState>,
) -> Result<Json<GithubAppClientIdResponse>, ApiError> {
    info!("get_github_app_client_id invoked");
    let client_id = state.config.github_app.client_id().to_string();

    Ok(Json(GithubAppClientIdResponse { client_id }))
}

pub async fn get_github_token(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
) -> Result<Json<GithubTokenResponse>, ApiError> {
    let response = actor.get_github_token(&state).await?;
    Ok(Json(response))
}
