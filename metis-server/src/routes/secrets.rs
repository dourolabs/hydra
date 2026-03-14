use super::resolve_username;
use crate::{
    app::AppState,
    domain::{actors::Actor, secrets::validate_secret_name, users::Username},
    store::ReadOnlyStore,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use metis_common::{
    ActorId,
    api::v1::{
        ApiError,
        secrets::{ListSecretsResponse, SetSecretRequest},
    },
};
use tracing::info;

/// Return 403 if the authenticated actor is not the requested user.
///
/// Only `ActorId::Username` actors are permitted; session and issue actors
/// are rejected even when their `creator` field matches the target.
fn authorize(actor: &Actor, target: &Username) -> Result<(), ApiError> {
    if let ActorId::Username(ref username) = actor.actor_id {
        if username.as_str() == target.as_str() {
            return Ok(());
        }
    }
    Err(ApiError::forbidden("you can only access your own secrets"))
}

/// GET /v1/users/:username/secrets
pub async fn list_secrets(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(username): Path<String>,
) -> Result<Json<ListSecretsResponse>, ApiError> {
    let username = resolve_username(&actor, &username)?;
    authorize(&actor, &username)?;

    info!(username = %username, "list_secrets invoked");

    let names = state
        .store
        .list_user_secret_names(&username)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "failed to list secrets");
            ApiError::internal(format!("failed to list secrets: {err}"))
        })?;

    info!(username = %username, count = names.len(), "list_secrets completed");
    Ok(Json(ListSecretsResponse { secrets: names }))
}

/// PUT /v1/users/:username/secrets/:name
pub async fn set_secret(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((username, name)): Path<(String, String)>,
    Json(payload): Json<SetSecretRequest>,
) -> Result<Json<()>, ApiError> {
    let username = resolve_username(&actor, &username)?;
    authorize(&actor, &username)?;

    if let Err(msg) = validate_secret_name(&name) {
        return Err(ApiError::bad_request(format!(
            "invalid secret name '{name}': {msg}"
        )));
    }

    info!(username = %username, secret_name = %name, "set_secret invoked");

    let encrypted = state
        .secret_manager
        .encrypt(&payload.value)
        .map_err(|err| {
            tracing::error!(error = %err, "failed to encrypt secret");
            ApiError::internal("failed to encrypt secret")
        })?;

    state
        .store
        .set_user_secret(&username, &name, &encrypted)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "failed to set secret");
            ApiError::internal(format!("failed to set secret: {err}"))
        })?;

    info!(username = %username, secret_name = %name, "set_secret completed");
    Ok(Json(()))
}

/// DELETE /v1/users/:username/secrets/:name
pub async fn delete_secret(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((username, name)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let username = resolve_username(&actor, &username)?;
    authorize(&actor, &username)?;

    if let Err(msg) = validate_secret_name(&name) {
        return Err(ApiError::bad_request(format!(
            "invalid secret name '{name}': {msg}"
        )));
    }

    info!(username = %username, secret_name = %name, "delete_secret invoked");

    state
        .store
        .delete_user_secret(&username, &name)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "failed to delete secret");
            ApiError::internal(format!("failed to delete secret: {err}"))
        })?;

    info!(username = %username, secret_name = %name, "delete_secret completed");
    Ok(Json(()))
}
