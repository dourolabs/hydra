use axum::{Json, extract::State};
use metis_common::api::v1::ApiError;
use serde::{Deserialize, Serialize};

use crate::app::AppState;

#[derive(Serialize, Deserialize)]
pub struct LocalAuthResponse {
    pub token: String,
}

pub async fn local_auth(
    State(state): State<AppState>,
) -> Result<Json<LocalAuthResponse>, ApiError> {
    let path = state.config.auth.auth_token_file().ok_or_else(|| {
        ApiError::bad_request("local-auth is only available when auth_token_file is configured")
    })?;

    let token = std::fs::read_to_string(path)
        .map_err(|_| ApiError::bad_request("auth token file not found or unreadable"))?
        .trim()
        .to_string();

    Ok(Json(LocalAuthResponse { token }))
}
