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
            let message = auth_failure_message(&store_error);
            info!(error = %store_error, "authorization rejected");
            return Err(ApiError::unauthorized(message));
        }
    };

    let mut actor = match state.get_actor(auth_token.actor_name()).await {
        Ok(actor) => actor,
        Err(error) => {
            let message = auth_failure_message(&error);
            info!(error = %error, "authorization rejected");
            return Err(ApiError::unauthorized(message));
        }
    };

    // Phase 3a (`/designs/actor-system-overhaul.md` §5.3): look up the
    // token row directly by its hash. The returned `session_id` is
    // hydrated onto `Actor.session_id` so `ActorRef::from(&actor)`
    // carries it into every downstream mutation.
    let token_hash = Actor::hash_auth_token(auth_token.raw_token());
    let matched_row = state
        .store()
        .get_auth_token_by_hash(&token_hash)
        .await
        .ok()
        .flatten()
        .filter(|row| row.actor_name == auth_token.actor_name());

    // Fall back to the legacy single-token hash on the Actor struct for
    // backward compatibility with tokens created before the auth_tokens
    // table existed. Phase 3b will delete this fallback.
    let token_valid = matched_row.is_some() || actor.verify_auth_token(&auth_token);
    if token_valid {
        actor.session_id = matched_row.and_then(|row| row.session_id);
        info!(actor = %actor.name(), "authorization accepted");
        request.extensions_mut().insert(actor);
        Ok(next.run(request).await)
    } else {
        let error = StoreError::InvalidAuthToken;
        let message = auth_failure_message(&error);
        info!(error = %error, "authorization rejected");
        Err(ApiError::unauthorized(message))
    }
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

fn auth_failure_message(error: &StoreError) -> &'static str {
    match error {
        StoreError::ActorNotFound(_)
        | StoreError::InvalidActorName(_)
        | StoreError::InvalidAuthToken => "authorization invalid",
        _ => "authorization unavailable",
    }
}

fn auth_token_error(error: &AuthTokenError) -> StoreError {
    match error {
        AuthTokenError::InvalidFormat => StoreError::InvalidAuthToken,
        AuthTokenError::InvalidActorName(name) => StoreError::InvalidActorName(name.clone()),
    }
}
