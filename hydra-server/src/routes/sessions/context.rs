use crate::{
    app::{AppState, rewrite_local_bundle_urls},
    domain::{actors::get_github_token_for_user, sessions::SessionMode},
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{Json, extract::State};
use hydra_common::{api::v1, constants::ENV_HYDRA_ID};
use tracing::{error, info, warn};

pub async fn get_session_context(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::WorkerContext>, ApiError> {
    info!(session_id = %session_id, "get_session_context invoked");

    let task = state.get_session(&session_id).await.map_err(|err| {
        error!(error = %err, session_id = %session_id, "failed to get task");
        ApiError::not_found(format!("Session '{session_id}' not found"))
    })?;

    // Build the API `Session` the worker will see. The persisted
    // `mount_spec` is now the single source of truth — no re-derivation
    // from a separate `context` / resolved-bundle path (PR-F).
    let mut session: v1::sessions::Session = task.clone().into();

    // When running in a containerized engine (e.g. Docker), rewrite file://
    // URLs inside `mount_spec.mounts[*].bundle` to the container-side mount
    // path so the worker receives URLs it can resolve from inside the
    // container.
    if state.job_engine.is_containerized() {
        let _ = rewrite_local_bundle_urls(&mut session.mount_spec);
    }

    // Resolve per-user secrets with global fallback and inject into env_vars
    // (the worker reads env_vars off `WorkerContext`).
    let mut env_vars = task.env_vars.clone();
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_HYDRA_ID.to_string(), session_id.to_string());
    session.env_vars = env_vars.clone();

    // For interactive sessions, fill in the server-configured idle-timeout
    // default into the embedded `SessionMode::Interactive` so the worker can
    // read it directly off `session.mode` without a separate handshake field.
    // Matches the domain `SessionMode` exhaustively so a new variant in a
    // future PR forces this site to be revisited (the cross-crate API
    // `SessionMode` is `#[non_exhaustive]`).
    if let SessionMode::Interactive { .. } = &task.mode {
        if let v1::sessions::SessionMode::Interactive {
            idle_timeout_secs, ..
        } = &mut session.mode
        {
            if idle_timeout_secs.is_none() {
                *idle_timeout_secs = Some(state.config.job.interactive_idle_timeout_secs);
            }
        }
    }

    // Resolve the creator's GitHub token server-side so the worker can clone
    // repos. Best-effort: if no token is on file we hand back `None` and the
    // worker fails at clone time with a clear auth error, matching the
    // pre-refactor `client.get_github_token().await.ok()` semantics.
    let github_token = match get_github_token_for_user(&state, &task.creator).await {
        Ok(response) => Some(response.github_token),
        Err(err) => {
            warn!(
                session_id = %session_id,
                creator = %task.creator,
                error = ?err,
                "no GitHub token for session creator; clone-needed sessions will fail"
            );
            None
        }
    };

    // `resumed_state` is reserved for the §3 resumption design; populated by a
    // future PR and read by the worker via `Session.resumed_from`.
    let context = v1::sessions::WorkerContext::new(session, env_vars, github_token, None);
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
