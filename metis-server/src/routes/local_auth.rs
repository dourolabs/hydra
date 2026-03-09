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
    let token = state
        .local_auth_token
        .as_ref()
        .ok_or_else(|| {
            ApiError::bad_request("local-auth is only available when auth_mode is 'local'")
        })?
        .clone();

    Ok(Json(LocalAuthResponse { token }))
}
