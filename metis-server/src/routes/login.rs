use crate::app::{AppState, LoginError};
use crate::routes::jobs::ApiError;
use axum::{Json, extract::State};
use metis_common::api::v1;
use tracing::{error, info};

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<v1::login::LoginRequest>,
) -> Result<Json<v1::login::LoginResponse>, ApiError> {
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    let github_refresh_token =
        normalize_non_empty("github_refresh_token", payload.github_refresh_token)?;
    info!("login invoked");

    let response = state
        .login_with_github_token(github_token, github_refresh_token)
        .await
        .map_err(map_login_error)?;

    info!(username = %response.user.username, "login completed");
    Ok(Json(response))
}

fn normalize_non_empty(field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(trimmed.to_string())
}

fn map_login_error(error: LoginError) -> ApiError {
    match error {
        LoginError::InvalidGithubToken(message) => {
            error!(error = %message, "login failed with invalid token");
            ApiError::bad_request("invalid GitHub token")
        }
        LoginError::ForbiddenGithubOrg { username } => {
            error!(username = %username, "login rejected by allowed orgs");
            ApiError::unauthorized("GitHub user is not in an allowed organization")
        }
        LoginError::Store { source } => {
            error!(error = %source, "login failed to store actor");
            ApiError::internal(format!("failed to login: {source}"))
        }
    }
}
