use crate::{
    app::{AppState, rewrite_local_bundle_url},
    domain::sessions::{BundleSpec, SessionMode},
    routes::sessions::{ApiError, SessionIdPath, mount_spec_from_create_request},
};
use axum::{Json, extract::State};
use hydra_common::{
    api::v1,
    constants::{ENV_HYDRA_ID, ENV_HYDRA_ISSUE_ID},
};
use tracing::{error, info};

pub async fn get_session_context(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::WorkerContext>, ApiError> {
    info!(session_id = %session_id, "get_session_context invoked");

    let task = state.get_session(&session_id).await.map_err(|err| {
        error!(error = %err, session_id = %session_id, "failed to get task");
        ApiError::not_found(format!("Session '{session_id}' not found"))
    })?;

    let mut resolved = state.resolve_task(&task).await.map_err(ApiError::from)?;

    // When running in a containerized engine (e.g. Docker), rewrite file:// URLs
    // to the container-side mount path so workers receive the correct URL.
    if state.job_engine.is_containerized() {
        rewrite_local_bundle_url(&mut resolved.context.bundle);
    }

    let mut env_vars = resolved.env_vars;
    state
        .resolve_secrets_into_env_vars(&task.creator, &mut env_vars, &task.secrets)
        .await;
    env_vars.insert(ENV_HYDRA_ID.to_string(), session_id.to_string());

    // Build the per-fetch MountSpec from the resolved Bundle. This is the
    // single source of truth for `CreateSessionRequest → MountSpec` (the
    // create-time builder in `app/sessions.rs::mount_spec_for_session` and
    // the migration backfill in `20260523020000_*` both mirror this shape).
    let bundle: v1::sessions::Bundle = resolved.context.bundle.clone().into();
    let service_repo_name = match &task.context {
        BundleSpec::ServiceRepository { name, .. } => Some(name.clone()),
        _ => None,
    };
    let issue_branch_id = env_vars.get(ENV_HYDRA_ISSUE_ID).cloned();
    let build_cache = match (service_repo_name, state.config.build_cache.to_context()) {
        (Some(name), Some(ctx)) => Some((name, ctx)),
        _ => None,
    };
    let mount_spec =
        mount_spec_from_create_request(bundle, session_id.clone(), issue_branch_id, build_cache);

    // Build the API `Session` that the worker will see. Start from the stored
    // task, then overlay the runtime-resolved env vars and the freshly-built
    // mount spec so the embedded Session matches what the worker will actually
    // run with. `resumed_state` plumbing is out of scope (PR-4 follow-up).
    let mut session: v1::sessions::Session = task.clone().into();
    session.env_vars = env_vars.clone();
    session.mount_spec = mount_spec.clone();

    // Legacy WorkerContext fields are populated from the session's mode.
    // Match against the *domain* `SessionMode` (exhaustive in-crate) rather
    // than the cross-crate `v1::sessions::SessionMode` (which is
    // `#[non_exhaustive]`, forcing a wildcard arm) so the compiler catches
    // any new variant added in PR-4. `task.mode` carries the same data the
    // embedded API session was just constructed from.
    let (prompt, interactive) = match &task.mode {
        SessionMode::Headless { prompt } => (prompt.clone(), None),
        SessionMode::Interactive {
            conversation_id,
            idle_timeout_secs,
            conversation_resume_from,
        } => (
            session
                .agent_config
                .system_prompt
                .clone()
                .unwrap_or_default(),
            Some(v1::sessions::InteractiveOptions::new(
                Some(conversation_id.clone()),
                Some(idle_timeout_secs.unwrap_or(state.config.job.interactive_idle_timeout_secs)),
                *conversation_resume_from,
            )),
        ),
    };

    let context = v1::sessions::WorkerContext::new(
        prompt,
        session.agent_config.model.clone(),
        env_vars,
        session.agent_config.mcp_config.clone(),
        interactive,
        mount_spec,
        Some(session),
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
