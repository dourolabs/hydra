//! Mint a per-conversation proxy cookie.
//!
//! `POST /v1/conversations/<cid>/proxy-auth` — Bearer-auth on the main
//! router. Validates that the authenticated actor is the conversation's
//! creator, resolves the currently-active session for the conversation,
//! and returns 204 with the proxy cookie set. If the conversation has no
//! active session (Idle), the mint returns 409 so the UI can prompt the
//! user to send a message first (which re-activates).

use crate::app::AppState;
use crate::domain::actors::Actor;
use crate::proxy::access::user_principal;
use crate::proxy::cookie::{DEFAULT_COOKIE_TTL_SECS, ProxyCookiePayload, ProxyTargetId};
use crate::routes::sessions::proxy_auth::build_set_cookie_response;
use crate::store::StoreError;
use axum::{Extension, extract::State, response::Response};
use hydra_common::api::v1::ApiError;
use tracing::{error, info};

use super::ConversationIdPath;

pub async fn mint_conversation_proxy_auth(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Response, ApiError> {
    info!(%conversation_id, "mint_conversation_proxy_auth invoked");

    let versioned = state
        .store()
        .get_conversation(&conversation_id, false)
        .await
        .map_err(|err| match err {
            StoreError::ConversationNotFound(_) => {
                ApiError::not_found(format!("conversation '{conversation_id}' not found"))
            }
            other => {
                error!(%conversation_id, error = %other, "failed to load conversation");
                ApiError::internal(format!("Failed to load conversation: {other}"))
            }
        })?;

    // `versioned.item.creator` is `domain::users::Username`, distinct from
    // the `hydra_common::users::Username` carried by `ActorId::User`. Compare
    // via `as_str()` so the two wrap-the-same-string types line up.
    let creator_match = user_principal(&actor.actor_id)
        .map(|u| u.as_str() == versioned.item.creator.as_str())
        .unwrap_or(false);
    if !creator_match {
        return Err(ApiError::forbidden(
            "actor does not have read access to this conversation".to_string(),
        ));
    }

    let session_id_at_mint = state
        .chat_relay_map
        .active_session_id(&conversation_id)
        .ok_or_else(|| {
            ApiError::conflict(format!(
                "conversation '{conversation_id}' has no active session; \
                 send a message to resume before minting a proxy cookie"
            ))
        })?;

    let target = ProxyTargetId::Conversation(conversation_id.clone());
    let payload = ProxyCookiePayload {
        actor_id: actor.actor_id.clone(),
        target: target.clone(),
        session_id_at_mint,
        exp: chrono::Utc::now().timestamp() + DEFAULT_COOKIE_TTL_SECS,
    };
    build_set_cookie_response(&state, &target, &payload)
}
