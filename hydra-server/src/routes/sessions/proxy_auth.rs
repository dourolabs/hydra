//! Mint a per-session proxy cookie.
//!
//! `POST /v1/sessions/<sid>/proxy-auth` — Bearer-auth on the main router.
//! Validates that the authenticated actor has read access to the session
//! (matches the creator, or — for an Interactive session — matches the
//! owning conversation's creator), then returns 204 with the proxy cookie
//! set.

use crate::app::AppState;
use crate::domain::actors::Actor;
use crate::domain::sessions::SessionMode;
use crate::proxy::cookie::{
    DEFAULT_COOKIE_TTL_SECS, ProxyCookiePayload, ProxyTargetId, cookie_name, mint,
};
use crate::store::StoreError;
use axum::{
    Extension,
    extract::State,
    http::{HeaderValue, StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Response},
};
use hydra_common::actor_ref::ActorId;
use hydra_common::api::v1::ApiError;
use tracing::{error, info};

use super::SessionIdPath;

pub async fn mint_session_proxy_auth(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Response, ApiError> {
    info!(session_id = %session_id, "mint_session_proxy_auth invoked");

    let session = state
        .get_session(&session_id)
        .await
        .map_err(|err| match err {
            StoreError::SessionNotFound(_) => {
                ApiError::not_found(format!("session '{session_id}' not found"))
            }
            other => {
                error!(%session_id, error = %other, "failed to load session");
                ApiError::internal(format!("Failed to load session: {other}"))
            }
        })?;

    // Read-access gate: the authenticated actor must match either the
    // session's creator or the owning conversation's creator (for
    // interactive sessions). The proxy gives the actor live reach into a
    // running dev process, so the same authority required to read the
    // conversation transcript governs the proxy.
    let actor_principal = actor_principal_name(&actor);
    let session_creator = session.creator.as_str();
    let is_session_creator = actor_principal == session_creator;
    let is_conversation_creator = match &session.mode {
        SessionMode::Interactive {
            conversation_id, ..
        } => match state.store().get_conversation(conversation_id, false).await {
            Ok(versioned) => actor_principal == versioned.item.creator.as_str(),
            Err(_) => false,
        },
        SessionMode::Headless => false,
    };

    if !is_session_creator && !is_conversation_creator {
        return Err(ApiError::forbidden(
            "actor does not have read access to this session".to_string(),
        ));
    }

    let target = ProxyTargetId::Session(session_id.clone());
    // For a session-id direct target, session_id_at_mint == the target
    // session itself. The proxy router's session-id-at-mint check is
    // trivially satisfied as long as the session is still alive.
    let payload = ProxyCookiePayload {
        actor_id: actor.actor_id.clone(),
        target: target.clone(),
        session_id_at_mint: session_id.clone(),
        exp: chrono::Utc::now().timestamp() + DEFAULT_COOKIE_TTL_SECS,
    };
    build_set_cookie_response(&state, &target, &payload)
}

pub(crate) fn build_set_cookie_response(
    state: &AppState,
    target: &ProxyTargetId,
    payload: &ProxyCookiePayload,
) -> Result<Response, ApiError> {
    let token = mint(&state.secret_manager, payload)
        .map_err(|e| ApiError::internal(format!("Failed to mint proxy cookie: {e}")))?;
    let name = cookie_name(target);
    let proxy_host = state.config.hydra.proxy_host.clone();
    let domain_attr = if proxy_host.is_empty() {
        String::new()
    } else {
        format!("; Domain=.{proxy_host}")
    };
    let cookie = format!(
        "{name}={token}; Path=/{domain_attr}; Secure; HttpOnly; SameSite=Lax; Max-Age={DEFAULT_COOKIE_TTL_SECS}",
    );

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|e| {
            ApiError::internal(format!("Failed to build proxy cookie header value: {e}"))
        })?,
    );
    Ok(response)
}

pub(crate) fn actor_principal_name(actor: &Actor) -> &str {
    // Sessions store the creator as a `Username`. Map back to a string for
    // the comparison; agent/external/adhoc actors are not creators of any
    // human-visible session, so they fail the equality check naturally.
    match &actor.actor_id {
        ActorId::User(u) => u.as_str(),
        ActorId::Agent(a) => a.as_str(),
        ActorId::Adhoc(s) => s.as_ref(),
        ActorId::External { username, .. } => username.as_str(),
    }
}
