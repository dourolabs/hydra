use crate::{
    app::AppState,
    domain::{actors::Actor, secrets::ALLOWED_SECRET_NAMES, users::Username},
    store::ReadOnlyStore,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use metis_common::api::v1::{
    ApiError,
    secrets::{ListSecretsResponse, SetSecretRequest},
};
use tracing::info;

/// Resolve the `:username` path parameter: "me" maps to the authenticated
/// user's username; any other value is returned as-is.
fn resolve_username(actor: &Actor, raw: &str) -> Result<Username, ApiError> {
    if raw == "me" {
        Ok(actor.creator.clone())
    } else {
        Ok(Username::from(raw.to_string()))
    }
}

/// Return 403 if the authenticated actor is not the requested user.
fn authorize(actor: &Actor, target: &Username) -> Result<(), ApiError> {
    if actor.creator.as_str() != target.as_str() {
        return Err(ApiError::forbidden("you can only access your own secrets"));
    }
    Ok(())
}

/// Return 503 if the SecretManager is not configured.
fn require_secret_manager(
    state: &AppState,
) -> Result<&std::sync::Arc<crate::domain::secrets::SecretManager>, ApiError> {
    state.secret_manager.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(
            "secret management is not configured (encryption key missing)",
        )
    })
}

/// GET /v1/users/:username/secrets
pub async fn list_secrets(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(username): Path<String>,
) -> Result<Json<ListSecretsResponse>, ApiError> {
    require_secret_manager(&state)?;
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
    let secret_manager = require_secret_manager(&state)?;
    let username = resolve_username(&actor, &username)?;
    authorize(&actor, &username)?;

    if !ALLOWED_SECRET_NAMES.contains(&name.as_str()) {
        return Err(ApiError::bad_request(format!(
            "unknown secret name '{name}'; allowed names: {}",
            ALLOWED_SECRET_NAMES.join(", ")
        )));
    }

    info!(username = %username, secret_name = %name, "set_secret invoked");

    let encrypted = secret_manager.encrypt(&payload.value).map_err(|err| {
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
    require_secret_manager(&state)?;
    let username = resolve_username(&actor, &username)?;
    authorize(&actor, &username)?;

    if !ALLOWED_SECRET_NAMES.contains(&name.as_str()) {
        return Err(ApiError::bad_request(format!(
            "unknown secret name '{name}'; allowed names: {}",
            ALLOWED_SECRET_NAMES.join(", ")
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
