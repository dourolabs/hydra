use crate::{
    app::AppState,
    domain::actors::{Actor, UserOrWorker},
    routes::jobs::ApiError,
    store::StoreError,
};
use axum::{Extension, Json, extract::State};
use metis_common::github::{GithubAppClientIdResponse, GithubTokenResponse};
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
    let store = state.store.read().await;

    let username = match &actor.user_or_worker {
        UserOrWorker::Username(username) => username.clone(),
        UserOrWorker::Task(task_id) => {
            let task = store.get_task(task_id).await.map_err(|err| match err {
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

            let issue = store.get_issue(&issue_id).await.map_err(|err| match err {
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

    let user = store.get_user(&username).await.map_err(|err| match err {
        StoreError::UserNotFound(missing) => {
            error!(username = %missing, "user not found");
            ApiError::not_found(format!("user '{missing}' not found"))
        }
        other => {
            error!(username = %username, error = %other, "failed to load user");
            ApiError::internal(format!("failed to load user '{username}': {other}"))
        }
    })?;

    info!(actor = %actor.name(), username = %username, "get_github_token completed");
    Ok(Json(GithubTokenResponse {
        github_token: user.github_token,
    }))
}
