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

    // When running in a containerized engine (e.g. Docker), rewrite file://
    // URLs inside `mount_spec.mounts[*].bundle` to the container-side mount
    // path so the worker receives URLs it can resolve from inside the
    // container.
    let mut mount_spec = task.mount_spec.clone();
    if state.job_engine.is_containerized() {
        let _ = rewrite_local_bundle_urls(&mut mount_spec);
    }

    // Resolve per-user secrets with global fallback and inject into env_vars.
    let mut env_vars = task.env_vars.clone();
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_HYDRA_ID.to_string(), session_id.to_string());

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

    // Project the mode kind + idle timeout. For interactive sessions, fall
    // back to the server-configured default when the caller didn't pin a
    // value at create-time.
    let (mode_kind, idle_timeout_secs) = match &task.mode {
        SessionMode::Headless => (v1::sessions::SessionModeKind::Headless, None),
        SessionMode::Interactive {
            idle_timeout_secs, ..
        } => (
            v1::sessions::SessionModeKind::Interactive,
            Some(idle_timeout_secs.unwrap_or(state.config.job.interactive_idle_timeout_secs)),
        ),
    };

    let context = v1::sessions::WorkerContext::new(
        session_id.clone(),
        mode_kind,
        mount_spec.mounts,
        mount_spec.working_dir,
        task.agent_config.model.clone(),
        task.agent_config.mcp_config.clone(),
        idle_timeout_secs,
        env_vars,
        github_token,
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
