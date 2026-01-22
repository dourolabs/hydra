use crate::domain::users::{
    CreateUserRequest, DeleteUserResponse, ListUsersResponse, ResolveUserRequest,
    ResolveUserResponse, UpdateGithubTokenRequest, UpsertUserResponse, User, UserSummary, Username,
};
use crate::{app::AppState, routes::jobs::ApiError, store::StoreError};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::api::v1;
use tracing::{error, info};

pub async fn list_users(
    State(state): State<AppState>,
) -> Result<Json<v1::users::ListUsersResponse>, ApiError> {
    info!("list_users invoked");
    let store = state.store.read().await;
    let users = store.list_users().await.map_err(|err| {
        error!(error = %err, "failed to list users");
        ApiError::internal(anyhow::anyhow!("failed to list users: {err}"))
    })?;

    let summaries = users.into_iter().map(UserSummary::from).collect::<Vec<_>>();
    info!(user_count = summaries.len(), "list_users completed");

    let response: v1::users::ListUsersResponse = ListUsersResponse::new(summaries).into();
    Ok(Json(response))
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<v1::users::CreateUserRequest>,
) -> Result<Json<v1::users::UpsertUserResponse>, ApiError> {
    let payload: CreateUserRequest = payload.into();
    let username: Username = normalize_non_empty("username", payload.username.into())?.into();
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    let github_user_id = normalize_optional_positive("github_user_id", payload.github_user_id)?;
    info!(username = %username, "create_user invoked");

    let user = User {
        username: username.clone(),
        github_user_id,
        github_token,
    };

    let mut store = state.store.write().await;
    store
        .add_user(user.clone())
        .await
        .map_err(|err| match err {
            StoreError::UserAlreadyExists(existing) => {
                error!(username = %existing, "user already exists");
                ApiError::conflict(format!("user '{existing}' already exists"))
            }
            other => {
                error!(username = %username, error = %other, "failed to create user");
                ApiError::internal(anyhow::anyhow!(
                    "failed to create user '{username}': {other}"
                ))
            }
        })?;

    info!(username = %username, "create_user completed");
    let response: v1::users::UpsertUserResponse =
        UpsertUserResponse::new(UserSummary::from(user)).into();
    Ok(Json(response))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<v1::users::DeleteUserResponse>, ApiError> {
    let username: Username = normalize_non_empty("username", username)?.into();
    info!(username = %username, "delete_user invoked");

    let mut store = state.store.write().await;
    store
        .delete_user(&username)
        .await
        .map_err(|err| match err {
            StoreError::UserNotFound(missing) => {
                error!(username = %missing, "user not found");
                ApiError::not_found(format!("user '{missing}' not found"))
            }
            other => {
                error!(username = %username, error = %other, "failed to delete user");
                ApiError::internal(anyhow::anyhow!(
                    "failed to delete user '{username}': {other}"
                ))
            }
        })?;

    info!(username = %username, "delete_user completed");
    let response: v1::users::DeleteUserResponse = DeleteUserResponse::new(username).into();
    Ok(Json(response))
}

pub async fn set_github_token(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(payload): Json<v1::users::UpdateGithubTokenRequest>,
) -> Result<Json<v1::users::UpsertUserResponse>, ApiError> {
    let payload: UpdateGithubTokenRequest = payload.into();
    let username: Username = normalize_non_empty("username", username)?.into();
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    let github_user_id = normalize_optional_positive("github_user_id", payload.github_user_id)?;
    info!(username = %username, "set_github_token invoked");

    let mut store = state.store.write().await;
    let updated = store
        .set_user_github_token(&username, github_token, github_user_id)
        .await
        .map_err(|err| match err {
            StoreError::UserNotFound(missing) => {
                error!(username = %missing, "user not found");
                ApiError::not_found(format!("user '{missing}' not found"))
            }
            other => {
                error!(username = %username, error = %other, "failed to update github token");
                ApiError::internal(anyhow::anyhow!(
                    "failed to update github token for '{username}': {other}"
                ))
            }
        })?;

    info!(username = %username, "set_github_token completed");
    let response: v1::users::UpsertUserResponse =
        UpsertUserResponse::new(UserSummary::from(updated)).into();
    Ok(Json(response))
}

pub async fn resolve_user(
    State(state): State<AppState>,
    Json(payload): Json<v1::users::ResolveUserRequest>,
) -> Result<Json<v1::users::ResolveUserResponse>, ApiError> {
    let payload: ResolveUserRequest = payload.into();
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    info!("resolve_user invoked");

    let store = state.store.read().await;
    let user = store
        .get_user_by_github_token(&github_token)
        .await
        .map_err(|err| match err {
            StoreError::UserNotFoundForToken => {
                ApiError::not_found("user not found for provided token".to_string())
            }
            other => {
                error!(error = %other, "failed to resolve user by token");
                ApiError::internal(anyhow::anyhow!("failed to resolve user by token: {other}"))
            }
        })?;

    info!(username = %user.username, "resolve_user completed");
    let response: v1::users::ResolveUserResponse =
        ResolveUserResponse::new(UserSummary::from(user)).into();
    Ok(Json(response))
}

fn normalize_non_empty(field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(trimmed.to_string())
}

fn normalize_optional_positive(field: &str, value: Option<u64>) -> Result<Option<u64>, ApiError> {
    match value {
        Some(0) => Err(ApiError::bad_request(format!(
            "{field} must be greater than zero"
        ))),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}
