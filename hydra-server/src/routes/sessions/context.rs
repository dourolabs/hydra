use crate::{
    app::{AppState, rewrite_local_bundle_url},
    domain::sessions::BundleSpec,
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{Json, extract::State};
use hydra_common::{
    api::v1,
    api::v1::sessions::{MountItem, MountSpec, RelativePath},
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

    let interactive = task.conversation_id().cloned().map(|conv_id| {
        v1::sessions::InteractiveOptions::new(
            Some(conv_id),
            Some(state.config.job.interactive_idle_timeout_secs),
            task.conversation_resume_from,
        )
    });

    let bundle: v1::sessions::Bundle = resolved.context.bundle.clone().into();
    let service_repo_name = match &task.context {
        BundleSpec::ServiceRepository { name, .. } => Some(name.clone()),
        _ => None,
    };
    let issue_branch_id = env_vars.get(ENV_HYDRA_ISSUE_ID).cloned();
    let repo_target = RelativePath::new("repo").expect("static `repo` path is valid");
    let docs_target = RelativePath::new("documents").expect("static `documents` path is valid");
    let mut mounts = Vec::with_capacity(3);
    mounts.push(MountItem::Bundle {
        target: repo_target.clone(),
        bundle,
        session_id: session_id.clone(),
        issue_branch_id,
    });
    if let (Some(name), Some(cache)) = (service_repo_name, state.config.build_cache.to_context()) {
        mounts.push(MountItem::BuildCache {
            repo_target: repo_target.clone(),
            service_repo_name: name,
            context: cache,
            session_id: session_id.clone(),
        });
    }
    mounts.push(MountItem::Documents {
        target: docs_target,
    });
    let mount_spec = MountSpec::new(repo_target, mounts);

    // Worker prompt: `SessionMode::Headless` carries it directly; for
    // `Interactive` mode it lives on `agent_config.system_prompt`.
    let wire_prompt = match &task.mode {
        crate::domain::sessions::SessionMode::Headless { prompt } => prompt.clone(),
        crate::domain::sessions::SessionMode::Interactive { .. } => {
            task.agent_config.system_prompt.clone().unwrap_or_default()
        }
    };
    let context = v1::sessions::WorkerContext::new(
        wire_prompt,
        task.agent_config.model.clone(),
        env_vars,
        task.agent_config.mcp_config.clone(),
        interactive,
        mount_spec,
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}
