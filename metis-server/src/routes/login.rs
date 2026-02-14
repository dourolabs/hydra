use crate::app::{AppState, LoginError};
use crate::domain::users::Username;
use crate::routes::jobs::ApiError;
use axum::{Json, extract::State};
use metis_common::api::v1;
use octocrab::Octocrab;
use serde::Deserialize;
use tracing::{error, info};

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<v1::login::LoginRequest>,
) -> Result<Json<v1::login::LoginResponse>, ApiError> {
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    let github_refresh_token =
        normalize_non_empty("github_refresh_token", payload.github_refresh_token)?;
    info!("login invoked");

    let github_client = Octocrab::builder()
        .base_uri(state.config.github_app.api_base_url().to_string())
        .map_err(|err| {
            error!(error = %err, "failed to parse github api base url");
            ApiError::internal(format!("failed to parse github api base url: {err}"))
        })?
        .personal_token(github_token.clone())
        .build()
        .map_err(|err| {
            error!(error = %err, "login failed with invalid token");
            ApiError::bad_request("invalid GitHub token")
        })?;

    let github_user = github_client.current().user().await.map_err(|err| {
        error!(error = %err, "login failed with invalid token");
        ApiError::bad_request("invalid GitHub token")
    })?;
    let username = Username::from(github_user.login);
    let github_user_id = github_user.id.into_inner();

    let allowed_orgs = &state.config.metis.allowed_orgs;
    if !allowed_orgs.is_empty() {
        #[derive(Deserialize)]
        struct GithubOrg {
            login: String,
        }

        let orgs: Vec<GithubOrg> =
            github_client
                .get("/user/orgs", None::<&()>)
                .await
                .map_err(|err| {
                    error!(error = %err, "login failed with invalid token");
                    ApiError::bad_request("invalid GitHub token")
                })?;

        let is_allowed = orgs.iter().any(|org| {
            allowed_orgs
                .iter()
                .any(|allowed| org.login.eq_ignore_ascii_case(allowed))
        });

        if !is_allowed {
            error!(username = %username, "login rejected by allowed orgs");
            return Err(ApiError::unauthorized(
                "GitHub user is not in an allowed organization",
            ));
        }
    }

    let response = state
        .login_with_github_token(username, github_user_id, github_token, github_refresh_token)
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
        LoginError::Store { source } => {
            error!(error = %source, "login failed to store actor");
            ApiError::internal(format!("failed to login: {source}"))
        }
    }
}
