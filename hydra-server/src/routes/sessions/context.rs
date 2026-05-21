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

    let build_cache = state.config.build_cache.to_context();
    let interactive = task.interactive.as_ref().map(|opts| {
        v1::sessions::InteractiveOptions::new(
            opts.conversation_id.clone(),
            Some(state.config.job.interactive_idle_timeout_secs),
            opts.conversation_resume_from,
        )
    });

    let bundle: v1::sessions::Bundle = resolved.context.bundle.clone().into();
    let service_repo_name = match &task.context {
        BundleSpec::ServiceRepository { name, .. } => Some(name.clone()),
        _ => None,
    };
    let issue_branch_id = env_vars.get(ENV_HYDRA_ISSUE_ID).cloned();
    let mount_spec = build_mount_spec(
        bundle.clone(),
        build_cache.clone(),
        service_repo_name,
        session_id.clone(),
        issue_branch_id,
    );

    let context = v1::sessions::WorkerContext::new(
        bundle,
        task.prompt,
        task.model.clone(),
        env_vars,
        build_cache,
        task.mcp_config.clone(),
        interactive,
        Some(mount_spec),
    );
    info!(session_id = %session_id, "get_session_context completed");
    Ok(Json(context))
}

/// Build the standard 3-item (or 2-item, when no build cache) mount spec for a
/// session. The order is `[Bundle, BuildCache?, Documents]`, matching the
/// gating in the worker's legacy `mounts::build_mounts`. `working_dir` is
/// always `"repo"` for the current standard layout.
fn build_mount_spec(
    bundle: v1::sessions::Bundle,
    build_cache: Option<hydra_common::BuildCacheContext>,
    service_repo_name: Option<hydra_common::RepoName>,
    session_id: hydra_common::SessionId,
    issue_branch_id: Option<String>,
) -> MountSpec {
    let repo_target = RelativePath::new("repo").expect("static `repo` path is valid");
    let docs_target = RelativePath::new("documents").expect("static `documents` path is valid");

    let mut mounts = Vec::with_capacity(3);
    mounts.push(MountItem::Bundle {
        target: repo_target.clone(),
        bundle,
        session_id: session_id.clone(),
        issue_branch_id,
    });
    if let (Some(name), Some(cache)) = (service_repo_name, build_cache) {
        mounts.push(MountItem::BuildCache {
            repo_target: repo_target.clone(),
            service_repo_name: name,
            context: cache,
            session_id,
        });
    }
    mounts.push(MountItem::Documents {
        target: docs_target,
    });
    MountSpec::new(repo_target, mounts)
}
