use crate::{
    app::AppState,
    config::GithubAppSection,
    domain::actors::{Actor, UserOrWorker},
    routes::jobs::ApiError,
    store::StoreError,
};
use axum::{Extension, Json, extract::State};
use metis_common::github::{GithubAppClientIdResponse, GithubTokenResponse};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, USER_AGENT},
};
use serde::Deserialize;
use tracing::{error, info};

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
    info!(actor = %actor.name(), "get_github_token invoked");
    let (username, user) = {
        let username = match &actor.user_or_worker {
            UserOrWorker::Username(username) => username.clone(),
            UserOrWorker::Task(task_id) => {
                let task = state.get_task(task_id).await.map_err(|err| match err {
                    StoreError::TaskNotFound(_) => {
                        error!(task_id = %task_id, "task not found");
                        ApiError::not_found(format!("task '{task_id}' not found"))
                    }
                    other => {
                        error!(task_id = %task_id, error = %other, "failed to load task");
                        ApiError::internal(format!("failed to load task '{task_id}': {other}"))
                    }
                })?;

                let issue_id = task.spawned_from.ok_or_else(|| {
                    error!(task_id = %task_id, "task missing spawned_from issue");
                    ApiError::not_found(format!("task '{task_id}' missing spawned_from issue"))
                })?;

                let issue = state.get_issue(&issue_id).await.map_err(|err| match err {
                    StoreError::IssueNotFound(_) => {
                        error!(issue_id = %issue_id, "issue not found");
                        ApiError::not_found(format!("issue '{issue_id}' not found"))
                    }
                    other => {
                        error!(issue_id = %issue_id, error = %other, "failed to load issue");
                        ApiError::internal(format!("failed to load issue '{issue_id}': {other}"))
                    }
                })?;

                issue.creator
            }
        };

        let user = state.get_user(&username).await.map_err(|err| match err {
            StoreError::UserNotFound(missing) => {
                error!(username = %missing, "user not found");
                ApiError::not_found(format!("user '{missing}' not found"))
            }
            other => {
                error!(username = %username, error = %other, "failed to load user");
                ApiError::internal(format!("failed to load user '{username}': {other}"))
            }
        })?;

        (username, user)
    };

    let mut github_token = user.github_token.clone();
    if !github_token_is_valid(&state.config.github_app, &github_token).await? {
        let refreshed =
            refresh_github_token(&state.config.github_app, &user.github_refresh_token).await?;
        let updated = state
            .set_user_github_token(
                &username,
                refreshed.access_token.clone(),
                user.github_user_id,
                refreshed.refresh_token.clone(),
            )
            .await
            .map_err(|err| match err {
                StoreError::UserNotFound(missing) => {
                    error!(username = %missing, "user not found");
                    ApiError::not_found(format!("user '{missing}' not found"))
                }
                other => {
                    error!(username = %username, error = %other, "failed to refresh github token");
                    ApiError::internal(format!(
                        "failed to refresh github token for '{username}': {other}"
                    ))
                }
            })?;

        github_token = updated.github_token;
    }

    info!(actor = %actor.name(), username = %username, "get_github_token completed");
    Ok(Json(GithubTokenResponse { github_token }))
}

#[derive(Debug, Deserialize)]
struct GithubRefreshTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

struct RefreshedGithubToken {
    access_token: String,
    refresh_token: String,
}

async fn github_token_is_valid(config: &GithubAppSection, token: &str) -> Result<bool, ApiError> {
    let url = join_url(config.api_base_url(), "/user");
    let response = Client::new()
        .get(url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, "metis-server")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to validate github token");
            ApiError::internal("failed to validate github token")
        })?;

    match response.status() {
        StatusCode::OK => Ok(true),
        StatusCode::UNAUTHORIZED => Ok(false),
        status => {
            error!(status = %status, "unexpected github token validation response");
            Err(ApiError::internal(
                "unexpected response while validating github token",
            ))
        }
    }
}

async fn refresh_github_token(
    config: &GithubAppSection,
    current_refresh_token: &str,
) -> Result<RefreshedGithubToken, ApiError> {
    let url = join_url(config.oauth_base_url(), "/login/oauth/access_token");
    let response = Client::new()
        .post(url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, "metis-server")
        .form(&[
            ("client_id", config.client_id()),
            ("client_secret", config.client_secret()),
            ("grant_type", "refresh_token"),
            ("refresh_token", current_refresh_token),
        ])
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to refresh github token");
            ApiError::internal("failed to refresh github token")
        })?;

    let status = response.status();
    let payload = response
        .json::<GithubRefreshTokenResponse>()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to decode github token refresh response");
            ApiError::internal("failed to decode github token refresh response")
        })?;

    if let Some(access_token) = payload.access_token {
        return Ok(RefreshedGithubToken {
            access_token,
            refresh_token: payload
                .refresh_token
                .unwrap_or_else(|| current_refresh_token.to_string()),
        });
    }

    let message = payload
        .error_description
        .or(payload.error)
        .unwrap_or_else(|| "github token refresh failed".to_string());

    error!(status = %status, error = %message, "github token refresh failed");
    Err(ApiError::unauthorized("GitHub token refresh failed"))
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}
