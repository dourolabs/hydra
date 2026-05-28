use crate::{
    app::{AppState, rewrite_local_bundle_urls},
    domain::{actors::get_github_token_for_user, sessions::SessionMode},
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{Json, extract::State};
use hydra_common::{
    api::v1,
    api::v1::sessions::{AgentConfigRuntime, SessionModeKind},
    constants::ENV_HYDRA_ID,
};
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

    // Compute the API mount_spec the worker will see, rewriting file://
    // URLs inside `mount_spec.mounts[*].bundle` to the container-side mount
    // path so the worker receives URLs it can resolve from inside the
    // container.
    let mut mount_spec = task.mount_spec.clone();
    if state.job_engine.is_containerized() {
        let _ = rewrite_local_bundle_urls(&mut mount_spec);
    }

    // Resolve per-user secrets with global fallback and inject into env_vars
    // (the worker reads env_vars off `WorkerContext`).
    let mut env_vars = task.env_vars.clone();
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_HYDRA_ID.to_string(), session_id.to_string());

    // Mode kind discriminator + per-mode idle-timeout default lookup.
    let (mode_kind, idle_timeout_secs) = match &task.mode {
        SessionMode::Headless { .. } => (SessionModeKind::Headless, None),
        SessionMode::Interactive {
            idle_timeout_secs, ..
        } => (
            SessionModeKind::Interactive,
            Some(idle_timeout_secs.unwrap_or(state.config.job.interactive_idle_timeout_secs)),
        ),
    };

    let agent_config_runtime = AgentConfigRuntime::new(
        task.agent_config.model.clone(),
        task.agent_config.mcp_config.clone(),
        idle_timeout_secs,
    );

    // Resolve the creator's GitHub token server-side so the worker can clone
    // repos. Best-effort: if no token is on file we hand back `None` and the
    // worker fails at clone time with a clear auth error.
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

    let context = v1::sessions::WorkerContext::new(
        session_id.clone(),
        mode_kind,
        mount_spec,
        agent_config_runtime,
        env_vars,
        github_token,
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
