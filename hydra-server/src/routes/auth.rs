use crate::{
    app::AppState,
    domain::actors::{Actor, AuthToken, AuthTokenError},
    routes::sessions::ApiError,
    store::StoreError,
};
use axum::{
    body::Body,
    extract::State,
    http::{Request, header},
    middleware::Next,
    response::Response,
};
use tracing::info;

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let token = match extract_bearer_token(request.headers()) {
        Ok(token) => token,
        Err(message) => {
            info!(reason = %message, "authorization rejected");
            return Err(ApiError::unauthorized(message));
        }
    };

    let auth_token = match AuthToken::parse(token) {
        Ok(auth_token) => auth_token,
        Err(error) => {
            let store_error = auth_token_error(&error);
            let message = store_error.auth_failure_message();
            info!(error = %store_error, "authorization rejected");
            return Err(ApiError::unauthorized(message));
        }
    };

    let token_hash = Actor::hash_auth_token(auth_token.raw_token());
    let matched_row = state
        .store()
        .get_auth_token_by_hash(&token_hash)
        .await
        .ok()
        .flatten()
        .filter(|row| row.actor_name == auth_token.actor_name());

    let Some(matched_row) = matched_row else {
        let error = StoreError::InvalidAuthToken;
        let message = error.auth_failure_message();
        info!(error = %error, "authorization rejected");
        return Err(ApiError::unauthorized(message));
    };

    if matched_row.is_revoked {
        // Token belonged to a session that has since been killed
        // (`sessions/kill` → `revoke_auth_tokens_for_session`). Reject
        // with the same `authorization invalid` message we use for any
        // unknown token so callers see a uniform 401.
        let error = StoreError::InvalidAuthToken;
        let message = error.auth_failure_message();
        info!(
            actor = %auth_token.actor_name(),
            session_id = ?matched_row.session_id,
            "authorization rejected: token revoked"
        );
        return Err(ApiError::unauthorized(message));
    }

    // Build the runtime `Actor` straight from the matched token row.
    // `actor_id` parses from the token's `actor_name`; `creator` is the
    // per-token denormalization on `auth_tokens.creator`.
    let actor_id = match Actor::parse_name(auth_token.actor_name()) {
        Ok(id) => id,
        Err(error) => {
            let store_error = StoreError::InvalidActorName(error.to_string());
            let message = store_error.auth_failure_message();
            info!(error = %store_error, "authorization rejected");
            return Err(ApiError::unauthorized(message));
        }
    };

    let actor = Actor {
        actor_id,
        creator: matched_row.creator,
        session_id: matched_row.session_id,
    };
    info!(actor = %actor.name(), "authorization accepted");
    request.extensions_mut().insert(actor);
    Ok(next.run(request).await)
}

fn extract_bearer_token(headers: &header::HeaderMap) -> Result<&str, &'static str> {
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or("authorization required")?;
    let value = value
        .to_str()
        .map_err(|_| "authorization header must be valid utf-8")?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or("authorization must use Bearer scheme")?;
    let token = token.trim();
    if token.is_empty() {
        return Err("authorization token must not be empty");
    }
    Ok(token)
}

fn auth_token_error(error: &AuthTokenError) -> StoreError {
    match error {
        AuthTokenError::InvalidFormat => StoreError::InvalidAuthToken,
        AuthTokenError::InvalidActorName(name) => StoreError::InvalidActorName(name.clone()),
    }
}
