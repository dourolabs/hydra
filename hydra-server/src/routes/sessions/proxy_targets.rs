use crate::app::AppState;
use crate::domain::actors::Actor;
use crate::domain::actors::ActorRef;
use crate::store::StoreError;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use hydra_common::SessionId;
use hydra_common::api::v1::sessions::{
    ListProxyTargetsResponse, ProxyTarget, UpsertProxyTargetRequest,
};
use tracing::{error, info};

use super::{ApiError, SessionIdPath};

/// Authorize the request: the bearer token must belong to the worker for
/// `session_id`. The auth middleware populates `actor.session_id` with the
/// session-id bound to the token row (see `domain::actors::Actor`); the
/// proxy-target list can only be edited by that same session.
fn require_worker_for_session(actor: &Actor, session_id: &SessionId) -> Result<(), ApiError> {
    match actor.session_id.as_ref() {
        Some(bound) if bound == session_id => Ok(()),
        _ => Err(ApiError::forbidden(
            "only the worker for this session can edit its proxy targets".to_string(),
        )),
    }
}

/// `GET /v1/sessions/:session_id/proxy-targets`
///
/// Returns the list of proxy targets the worker has advertised. Readable by
/// any caller authorized to view the session.
pub async fn list_proxy_targets(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<ListProxyTargetsResponse>, ApiError> {
    info!(session_id = %session_id, "list_proxy_targets invoked");
    let session = state
        .get_session(&session_id)
        .await
        .map_err(|err| map_load_error(&session_id, err))?;
    Ok(Json(ListProxyTargetsResponse {
        targets: session.proxy_targets,
    }))
}

/// `POST /v1/sessions/:session_id/proxy-targets`
///
/// Adds (or replaces, when the port already exists) a proxy target on the
/// session. Idempotent — re-posting the same `port` replaces `ready_path`.
pub async fn upsert_proxy_target(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
    Json(payload): Json<UpsertProxyTargetRequest>,
) -> Result<StatusCode, ApiError> {
    info!(
        session_id = %session_id,
        port = payload.port,
        "upsert_proxy_target invoked"
    );
    require_worker_for_session(&actor, &session_id)?;
    let actor_ref = ActorRef::from(&actor);

    let mut session = state
        .get_session(&session_id)
        .await
        .map_err(|err| map_load_error(&session_id, err))?;

    let new = ProxyTarget {
        port: payload.port,
        ready_path: payload.ready_path,
    };
    match session
        .proxy_targets
        .iter_mut()
        .find(|t| t.port == new.port)
    {
        Some(existing) => *existing = new,
        None => session.proxy_targets.push(new),
    }

    state
        .store
        .update_session_with_actor(&session_id, session, actor_ref)
        .await
        .map_err(|err| map_update_error(&session_id, err))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/sessions/:session_id/proxy-targets/:port`
///
/// Removes the proxy target for `port`. Idempotent — deleting an absent
/// port returns 204.
pub async fn delete_proxy_target(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((session_id, port)): Path<(SessionId, u16)>,
) -> Result<StatusCode, ApiError> {
    info!(
        session_id = %session_id,
        port,
        "delete_proxy_target invoked"
    );
    require_worker_for_session(&actor, &session_id)?;
    let actor_ref = ActorRef::from(&actor);

    let mut session = state
        .get_session(&session_id)
        .await
        .map_err(|err| map_load_error(&session_id, err))?;

    let before = session.proxy_targets.len();
    session.proxy_targets.retain(|t| t.port != port);
    if session.proxy_targets.len() == before {
        // No change: nothing to persist. Idempotent.
        return Ok(StatusCode::NO_CONTENT);
    }

    state
        .store
        .update_session_with_actor(&session_id, session, actor_ref)
        .await
        .map_err(|err| map_update_error(&session_id, err))?;

    Ok(StatusCode::NO_CONTENT)
}

fn map_load_error(session_id: &SessionId, err: StoreError) -> ApiError {
    match err {
        StoreError::SessionNotFound(_) => {
            ApiError::not_found(format!("session '{session_id}' not found"))
        }
        other => {
            error!(%session_id, error = %other, "failed to load session");
            ApiError::internal(format!("Failed to load session '{session_id}': {other}"))
        }
    }
}

fn map_update_error(session_id: &SessionId, err: StoreError) -> ApiError {
    error!(%session_id, error = %err, "failed to persist proxy_targets update");
    ApiError::internal(format!("Failed to update session '{session_id}': {err}"))
}
