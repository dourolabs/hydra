use crate::{
    app::{AppState, rewrite_local_bundle_url},
    domain::sessions::SessionMode,
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
    let service_repo_name = task.service_repo_name().cloned();
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
    // run with. The worker reads `prompt` / `model` / `mcp_config` / mode
    // settings off this embedded value — there is no more legacy WorkerContext
    // duplication of those fields.
    let mut session: v1::sessions::Session = task.clone().into();
    session.env_vars = env_vars.clone();
    session.mount_spec = mount_spec;
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

    // `resumed_state` plumbing is a follow-up to Phase D step 15 (design §3.2);
    // until then the field is always `None`.
    let context = v1::sessions::WorkerContext::new(session, env_vars, None, None);
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
