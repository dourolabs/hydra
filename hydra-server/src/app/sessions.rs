use crate::{
    app::agents::AgentError,
    config::non_empty,
    domain::{
        actors::ActorRef,
        issues::SessionSettings,
        sessions::{AgentConfig, SessionMode},
        users::Username,
    },
    job_engine::{BindMount, JobEngineError, JobStatus},
    store::{ReadOnlyStore, Session, Status, StoreError, TaskError, TaskStatusLog},
};
use chrono::{DateTime, Duration, Utc};
use hydra_common::api::v1::sessions::{AgentSpec, Bundle, MountItem, MountSpec};
use hydra_common::{
    SessionId, Versioned,
    api::v1 as api,
    api::v1::sessions::SearchSessionsQuery,
    issues::IssueId,
    session_status::{SessionStatusUpdate, SetSessionStatusResponse},
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use tracing::{error, info, warn};

use super::TaskResolutionError;
use super::app_state::AppState;
use crate::policy::automations::agent_queue::AGENT_NAME_ENV_VAR;

pub(crate) const WORKER_NAME_SESSION_LIFECYCLE: &str = "session_lifecycle";
pub(crate) const WORKER_NAME_CLEANUP_ORPHANED_SESSIONS: &str = "cleanup_orphaned_sessions";

#[derive(Debug, Error)]
pub enum CreateSessionError {
    #[error(transparent)]
    TaskResolution(#[from] TaskResolutionError),
    #[error("failed to store session")]
    Store {
        #[source]
        source: StoreError,
    },
    /// Server-side resolution of a `Named` agent failed because no agent
    /// is registered under that name.
    #[error("agent '{name}' not found")]
    AgentNotFound { name: String },
    /// A bug in the store (or in the route layer) surfaced while loading
    /// the named agent — not a 404 case.
    #[error("failed to look up named agent")]
    AgentLookup {
        #[source]
        source: AgentError,
    },
    /// Reading the agent's prompt document failed (path empty / not found
    /// / store error). Carries the four-level resolver's error from
    /// [`AppState::resolve_session_system_prompt`] — only the agent layer
    /// hard-fails; project / status / system layers tolerate misses.
    #[error("failed to resolve prompt for agent '{name}': {source}")]
    AgentPromptResolution {
        name: String,
        #[source]
        source: anyhow::Error,
    },
    /// Reading the agent's MCP config document failed.
    #[error("failed to resolve mcp_config for agent '{name}': {source}")]
    AgentMcpConfigResolution {
        name: String,
        #[source]
        source: anyhow::Error,
    },
    /// `CreateSessionRequest.agent_config` carried a future-only
    /// `AgentSpec` variant that this server does not know how to lower.
    #[error("unsupported AgentSpec variant: {debug}")]
    UnsupportedAgentSpec { debug: String },
}

#[derive(Debug, Error)]
pub enum SetSessionStatusError {
    #[error("session '{session_id}' not found in store")]
    NotFound {
        #[source]
        source: StoreError,
        session_id: SessionId,
    },
    #[error("invalid status transition for session '{session_id}'")]
    InvalidStatusTransition { session_id: SessionId },
    #[error("failed to update status for session '{session_id}'")]
    Store {
        #[source]
        source: StoreError,
        session_id: SessionId,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
}

/// Merge agent-level secrets with the `session_settings.secrets` list,
/// deduplicating while preserving first-seen order. Returns `None` when
/// both inputs are empty so callers can drop the field from the
/// `CreateSessionRequest` rather than send an explicit empty `Vec`.
pub(crate) fn merge_agent_and_settings_secrets(
    agent: &crate::domain::agents::Agent,
    settings: &SessionSettings,
) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for s in agent
        .secrets
        .iter()
        .chain(settings.secrets.iter().flatten())
    {
        if seen.insert(s.clone()) {
            merged.push(s.clone());
        }
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

impl AppState {
    pub async fn add_session(
        &self,
        session: Session,
        created_at: DateTime<Utc>,
        actor: ActorRef,
    ) -> Result<SessionId, StoreError> {
        let (session_id, _version) = self
            .store
            .add_session_with_actor(session, created_at, actor)
            .await?;
        Ok(session_id)
    }

    /// Create a session from a fully-populated `CreateSessionRequest`.
    ///
    /// Callers own every field outside `agent_config` — `create_session`
    /// performs no derivation from `spawned_from` / `conversation_id`.
    /// Defaulting still happens for `model` (via `apply_session_settings_defaults`
    /// on a request-derived `SessionSettings`), and the `AgentSpec::Named`
    /// branch performs the agent/prompt/mcp_config/secrets resolution on
    /// the server side so callers can collapse to a one-line
    /// `agent_config: AgentSpec::Named { name }` request.
    ///
    /// Returns the assigned `SessionId` and the persisted domain `Session`
    /// so HTTP callers can populate `CreateSessionResponse.session` without
    /// a follow-up `GET /v1/sessions/:id`.
    pub async fn create_session(
        &self,
        request: api::sessions::CreateSessionRequest,
        actor: ActorRef,
        creator: Username,
    ) -> Result<(SessionId, Session), CreateSessionError> {
        let api::sessions::CreateSessionRequest {
            mode: req_mode,
            agent_config: req_agent_config,
            model: req_model,
            mount_spec: req_mount_spec,
            image: req_image,
            env_vars: req_env_vars,
            cpu_limit: req_cpu_limit,
            memory_limit: req_memory_limit,
            secrets: req_secrets,
            spawned_from,
            resumed_from,
            ..
        } = request;

        // Apply global defaults via the single `SessionSettings` ->
        // `apply_session_settings_defaults` path. Today the defaulter
        // only fills in `model` (from `config.job.default_model`), so
        // we only need to round-trip that field; the other request
        // values pass through unchanged.
        let model = self
            .apply_session_settings_defaults(SessionSettings {
                model: req_model,
                ..SessionSettings::default()
            })
            .model;
        let image = req_image;
        let cpu_limit = req_cpu_limit;
        let mut env_vars = req_env_vars;
        let memory_limit = req_memory_limit;
        let mut secrets = req_secrets;

        // Dispatch on the AgentSpec variant. `Named` reaches into the
        // store for the agent's prompt / mcp_config / secrets and stamps
        // the `AGENT_NAME_ENV_VAR` env var. `Adhoc` consumes the inline
        // prompt verbatim with no I/O.
        let agent_config = match req_agent_config {
            AgentSpec::Named { name } => {
                let agent = self
                    .get_agent(name.as_str())
                    .await
                    .map_err(|err| match err {
                        AgentError::NotFound { name } => CreateSessionError::AgentNotFound { name },
                        other => CreateSessionError::AgentLookup { source: other },
                    })?;
                let (project, status) = self.resolve_prompt_layers_for(spawned_from.as_ref()).await;
                let system_prompt = self
                    .resolve_session_system_prompt(&agent, &project, &status)
                    .await
                    .map_err(|source| CreateSessionError::AgentPromptResolution {
                        name: agent.name.clone(),
                        source,
                    })?;
                let mcp_config = match agent.mcp_config_path.as_deref() {
                    Some(path) => self
                        .resolve_agent_mcp_config(path)
                        .await
                        .map_err(|source| CreateSessionError::AgentMcpConfigResolution {
                            name: agent.name.clone(),
                            source,
                        })?,
                    None => None,
                };

                // Merge agent.secrets into the request's secrets while
                // preserving first-seen order. `merge_agent_and_settings_secrets`
                // walks `agent.secrets` then `settings.secrets`, so we
                // stage the request-supplied secrets as `settings.secrets`
                // before the merge to keep the agent-first ordering.
                let settings_with_secrets = SessionSettings {
                    secrets: secrets.clone(),
                    ..SessionSettings::default()
                };
                secrets = merge_agent_and_settings_secrets(&agent, &settings_with_secrets);

                env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), agent.name.clone());

                AgentConfig::new(Some(name), model, Some(system_prompt), mcp_config)
            }
            AgentSpec::Adhoc {
                system_prompt,
                mcp_config,
            } => AgentConfig::new(None, model, Some(system_prompt), mcp_config),
            other => {
                return Err(CreateSessionError::UnsupportedAgentSpec {
                    debug: format!("{other:?}"),
                });
            }
        };

        // Construction-time invariant: the spec must include a Bundle mount
        // that materializes `working_dir` on disk. A request that arrives
        // here with an empty `mount_spec` — chat sessions and PM/breakdown
        // sessions on a parent issue without a repository — would otherwise
        // hand the worker a `working_dir = "repo"` that nothing creates, and
        // `current_dir()` would ENOENT at spawn. Fall through to a
        // `Bundle::None` spec via `mount_spec_from_create_request`, which
        // the worker's `BundleMount` materializes as an empty directory at
        // setup time.
        use crate::routes::sessions::mount_spec_from_create_request;
        let mount_spec = if req_mount_spec.is_empty() {
            mount_spec_from_create_request(Bundle::None, None)
        } else {
            req_mount_spec
        };

        let mode: SessionMode = req_mode.into();

        let mut session = Session::new(
            creator,
            spawned_from,
            resumed_from,
            agent_config,
            mount_spec,
            image,
            env_vars,
            cpu_limit,
            memory_limit,
            secrets,
            mode,
            Status::Created,
            None,
            None,
        );

        self.resolve_task(&session).await?;

        let creation_time = Utc::now();
        let (session_id, _version) = self
            .store
            .add_session_with_actor(session.clone(), creation_time, actor)
            .await
            .map_err(|source| CreateSessionError::Store { source })?;

        // Populate the assigned creation_time so the returned `Session`
        // matches what the store now holds — saves callers a follow-up
        // `get_session` and lets the route response carry the canonical row.
        session.creation_time = Some(creation_time);

        Ok((session_id, session))
    }

    /// Resolve the `(Project, StatusDefinition)` pair feeding the four-level
    /// prompt resolver for a new session.
    ///
    /// - `Some(issue_id)`: load the issue, resolve its status via
    ///   [`AppState::resolve_status`], and load the project from the store.
    ///   Issues with `project_id = None` (residual shape; the
    ///   `seed_default_project` migration backfills production rows) are
    ///   resolved through the seeded default project. On any lookup
    ///   failure we fall back to the no-project sentinel — the session
    ///   still spawns, and the empty project / status slices keep
    ///   `system_prompt` byte-identical to today's `resolve_agent_prompt`
    ///   output. This matches the "tolerate missing layers" invariant
    ///   from the design doc.
    /// - `None`: conversation sessions and other issue-less spawns get the
    ///   [`no_project_sentinel`] (both `prompt_path = None`), so the
    ///   resolver emits system + agent only.
    async fn resolve_prompt_layers_for(
        &self,
        spawned_from: Option<&IssueId>,
    ) -> (
        hydra_common::api::v1::projects::Project,
        hydra_common::api::v1::projects::StatusDefinition,
    ) {
        use crate::domain::projects::{default_project_id, no_project_sentinel};

        let Some(issue_id) = spawned_from else {
            return no_project_sentinel();
        };
        let issue = match self.get_issue(issue_id, false).await {
            Ok(v) => v.item,
            Err(err) => {
                info!(
                    issue_id = %issue_id,
                    error = %err,
                    "could not load spawned_from issue for prompt layering; using no-project sentinel"
                );
                return no_project_sentinel();
            }
        };
        let status = match self.resolve_status(&issue).await {
            Ok(def) => def,
            Err(err) => {
                info!(
                    issue_id = %issue_id,
                    error = %err,
                    "could not resolve status for prompt layering; using no-project sentinel"
                );
                return no_project_sentinel();
            }
        };
        let project_id = issue.project_id.clone().unwrap_or_else(default_project_id);
        let project = match self.store.as_ref().get_project(&project_id, false).await {
            Ok(v) => v.item,
            Err(err) => {
                info!(
                    issue_id = %issue_id,
                    project_id = %project_id,
                    error = %err,
                    "could not load project for prompt layering; using no-project sentinel"
                );
                return no_project_sentinel();
            }
        };
        (project, status)
    }

    pub(crate) fn apply_session_settings_defaults(
        &self,
        mut settings: SessionSettings,
    ) -> SessionSettings {
        if settings.model.is_none() {
            if let Some(default_model) =
                self.config.job.default_model.as_deref().and_then(non_empty)
            {
                settings.model = Some(default_model.to_string());
            }
        }

        settings
    }

    /// Lower a `SessionSettings` into a fully-resolved `MountSpec`.
    ///
    /// Used by callers (`agent_queue::build_task`,
    /// `spawn_conversation_sessions::spawn_session`) that own the
    /// `CreateSessionRequest` and need to pre-populate its `mount_spec`
    /// since `create_session` no longer derives one from session_settings.
    ///
    /// Empty `remote_url` / missing `repo_name` returns the default
    /// (empty) spec; the server's `create_session` then falls through
    /// to a `Bundle::None` spec via `mount_spec_from_create_request`.
    pub(crate) async fn resolve_mount_spec(
        &self,
        settings: &SessionSettings,
    ) -> Result<MountSpec, crate::store::StoreError> {
        use crate::routes::sessions::mount_spec_from_create_request;

        let (bundle, service_repo_name) =
            match (settings.remote_url.as_ref(), settings.repo_name.as_ref()) {
                (Some(remote_url), repo_name) if !remote_url.trim().is_empty() => {
                    let rev = settings
                        .branch
                        .clone()
                        .unwrap_or_else(|| "main".to_string());
                    let bundle = Bundle::GitRepository {
                        url: remote_url.trim().to_string(),
                        rev,
                    };
                    (bundle, repo_name.cloned())
                }
                (_, Some(repo_name)) => {
                    let repository = self.repository_from_store(repo_name).await?;
                    let rev = settings
                        .branch
                        .clone()
                        .or_else(|| repository.default_branch.clone())
                        .unwrap_or_else(|| "main".to_string());
                    let bundle = Bundle::GitRepository {
                        url: repository.remote_url.clone(),
                        rev,
                    };
                    (bundle, Some(repo_name.clone()))
                }
                _ => return Ok(MountSpec::default()),
            };

        let build_cache = match (service_repo_name, self.config.build_cache.to_context()) {
            (Some(name), Some(ctx)) => Some((name, ctx)),
            _ => None,
        };
        Ok(mount_spec_from_create_request(bundle, build_cache))
    }

    pub async fn set_session_status(
        &self,
        session_id: SessionId,
        status: SessionStatusUpdate,
        actor: ActorRef,
    ) -> Result<SetSessionStatusResponse, SetSessionStatusError> {
        {
            let store = self.store.as_ref();

            store
                .get_session(&session_id, false)
                .await
                .map(|_| ())
                .map_err(|source| SetSessionStatusError::NotFound {
                    source,
                    session_id: session_id.clone(),
                })?;

            self.transition_task_to_completion(
                &session_id,
                status.to_result().map_err(|e| {
                    TaskError::try_from(e).unwrap_or_else(|err| TaskError::JobEngineError {
                        reason: format!("unknown task error: {err}"),
                    })
                }),
                status.last_message(),
                status.usage(),
                actor,
            )
            .await
            .map_err(|source| match source {
                StoreError::InvalidStatusTransition => {
                    SetSessionStatusError::InvalidStatusTransition {
                        session_id: session_id.clone(),
                    }
                }
                other => SetSessionStatusError::Store {
                    source: other,
                    session_id: session_id.clone(),
                },
            })?;
        }

        Ok(SetSessionStatusResponse::new(
            session_id,
            status.as_status(),
        ))
    }

    /// Loads all user secrets and injects them as env vars, then falls back to config
    /// values for system secrets (OPENAI_API_KEY, ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN)
    /// not already set.
    pub(crate) async fn resolve_secrets_into_env_vars(
        &self,
        creator: &Username,
        env_vars: &mut HashMap<String, String>,
        secrets_filter: &Option<Vec<String>>,
    ) {
        use hydra_common::constants::{
            ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
        };

        const AI_MODEL_KEYS: &[&str] = &[
            ENV_OPENAI_API_KEY,
            ENV_ANTHROPIC_API_KEY,
            ENV_CLAUDE_CODE_OAUTH_TOKEN,
        ];

        info!(
            username = %creator,
            ?secrets_filter,
            "resolving secrets for user"
        );

        // 1. Load user secrets and inject them as env vars, filtered by Task.secrets.
        let user_secret_names = match self.store.list_user_secret_names(creator).await {
            Ok(names) => names,
            Err(err) => {
                warn!(
                    username = %creator,
                    error = %err,
                    "failed to list user secret names"
                );
                Vec::new()
            }
        };

        info!(
            username = %creator,
            user_secrets_count = user_secret_names.len(),
            ?user_secret_names,
            "found user secrets"
        );

        for secret_ref in &user_secret_names {
            let secret_name = &secret_ref.name;
            // Always inject well-known AI model keys; only inject other secrets
            // if they appear in the task's secrets filter.
            let is_ai_key = AI_MODEL_KEYS.contains(&secret_name.as_str());
            if !is_ai_key {
                let allowed = secrets_filter
                    .as_ref()
                    .is_some_and(|filter| filter.contains(secret_name));
                if !allowed {
                    info!(
                        username = %creator,
                        secret = %secret_name,
                        "secret filtered out (not in secrets_filter and not an AI key)"
                    );
                    continue;
                }
            }

            match self.store.get_user_secret(creator, secret_name).await {
                Ok(Some(encrypted)) => match self.secret_manager.decrypt(&encrypted) {
                    Ok(value) => {
                        info!(
                            username = %creator,
                            secret = %secret_name,
                            "successfully decrypted and set user secret"
                        );
                        env_vars.insert(secret_name.clone(), value);
                    }
                    Err(err) => {
                        warn!(
                            username = %creator,
                            secret = %secret_name,
                            error = %err,
                            "failed to decrypt user secret, skipping"
                        );
                    }
                },
                Ok(None) => {
                    info!(
                        username = %creator,
                        secret = %secret_name,
                        "no secret found in store for this name"
                    );
                }
                Err(err) => {
                    warn!(
                        username = %creator,
                        secret = %secret_name,
                        error = %err,
                        "failed to look up user secret, skipping"
                    );
                }
            }
        }

        // 2. For system secrets not already set by user secrets, fall back to config.
        let system_entries: [(&str, Option<&str>); 3] = [
            (
                ENV_OPENAI_API_KEY,
                self.config.hydra.openai_api_key.as_deref(),
            ),
            (
                ENV_ANTHROPIC_API_KEY,
                self.config.hydra.anthropic_api_key.as_deref(),
            ),
            (
                ENV_CLAUDE_CODE_OAUTH_TOKEN,
                self.config.hydra.claude_code_oauth_token.as_deref(),
            ),
        ];

        for (secret_name, config_fallback) in system_entries {
            if env_vars.contains_key(secret_name) {
                info!(
                    username = %creator,
                    secret = secret_name,
                    source = "user",
                    "system secret resolved from user override"
                );
                continue;
            }

            let global_value = config_fallback
                .map(str::to_string)
                .filter(|v| !v.trim().is_empty());

            if let Some(value) = global_value {
                info!(
                    username = %creator,
                    secret = secret_name,
                    source = "config",
                    "system secret resolved from config fallback"
                );
                env_vars.insert(secret_name.to_string(), value);
            } else {
                info!(
                    username = %creator,
                    secret = secret_name,
                    source = "none",
                    "system secret not available from user or config"
                );
            }
        }

        // 3. Auto-inject GH_TOKEN from the creator's GitHub OAuth token when
        //    requested in secrets_filter and not already set by user secrets.
        use crate::domain::{actors::get_github_token_for_user, secrets::SECRET_GH_TOKEN};

        let gh_token_requested = secrets_filter
            .as_ref()
            .is_some_and(|filter| filter.iter().any(|s| s == SECRET_GH_TOKEN));

        if gh_token_requested && !env_vars.contains_key(SECRET_GH_TOKEN) {
            match get_github_token_for_user(self, creator).await {
                Ok(response) => {
                    env_vars.insert(SECRET_GH_TOKEN.to_string(), response.github_token);
                    info!(
                        username = %creator,
                        "GH_TOKEN auto-injected from creator's GitHub OAuth token"
                    );
                }
                Err(err) => {
                    warn!(
                        username = %creator,
                        error = ?err,
                        "failed to auto-inject GH_TOKEN from creator's GitHub OAuth token, skipping"
                    );
                }
            }
        }
    }

    pub async fn start_pending_task(&self, session_id: SessionId, actor: ActorRef) {
        let job_config = self.config.job.clone();
        let (mut resolved, cpu_limit, memory_limit, creator, secrets) = {
            let store = self.store.as_ref();
            match store.get_session(&session_id, false).await {
                Ok(versioned) => match self.resolve_task(&versioned.item).await {
                    Ok(resolved) => (
                        resolved,
                        versioned.item.cpu_limit.clone(),
                        versioned.item.memory_limit.clone(),
                        versioned.item.creator.clone(),
                        versioned.item.secrets.clone(),
                    ),
                    Err(err) => {
                        warn!(
                            hydra_id = %session_id,
                            error = %err,
                            "failed to resolve task for spawning"
                        );
                        return;
                    }
                },
                Err(err) => {
                    warn!(
                        hydra_id = %session_id,
                        error = %err,
                        "failed to load task for spawning"
                    );
                    return;
                }
            }
        };

        // Resolve per-user secrets with global fallback and inject into env_vars.
        self.resolve_secrets_into_env_vars(&creator, &mut resolved.env_vars, &secrets)
            .await;

        let cpu_limit = cpu_limit.unwrap_or_else(|| job_config.cpu_limit.clone());
        let memory_limit = memory_limit.unwrap_or_else(|| job_config.memory_limit.clone());
        let cpu_request = job_config.cpu_request.clone();
        let memory_request = job_config.memory_request.clone();

        let (task_actor, auth_token) = match self.create_actor_for_job(session_id.clone()).await {
            Ok(values) => values,
            Err(err) => {
                let failure_reason = format!("Failed to create actor for task: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &session_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        None,
                        actor,
                    )
                    .await
                {
                    error!(
                        hydra_id = %session_id,
                        error = %update_err,
                        "failed to set task status to Failed (actor creation failed)"
                    );
                } else {
                    info!(
                        hydra_id = %session_id,
                        "set task status to Failed (actor creation failed)"
                    );
                }
                return;
            }
        };

        // Detect file:// URLs and construct bind mounts for Docker containers.
        // Rewrites the bundle URL in-place to the container-side mount path.
        let bind_mounts = build_bind_mounts_for_local_repo(&mut resolved.context.bundle);

        match self
            .job_engine
            .create_job(
                &session_id,
                &task_actor,
                &auth_token,
                &resolved.image,
                &resolved.env_vars,
                cpu_limit,
                memory_limit,
                cpu_request,
                memory_request,
                bind_mounts,
            )
            .await
        {
            Ok(()) => match self
                .transition_task_to_pending(&session_id, actor.clone())
                .await
            {
                Ok(_) => {
                    info!(
                        hydra_id = %session_id,
                        "set task status to Pending (spawned)"
                    );
                }
                Err(err) => {
                    warn!(
                        hydra_id = %session_id,
                        error = %err,
                        "failed to set task to Pending after spawn"
                    );
                }
            },
            Err(err) => {
                // For non-AlreadyExists errors (e.g. etcdserver timeouts), the job
                // may have actually been created despite the error. Wait briefly for
                // etcd to settle, then check whether the job exists in K8s before
                // marking the task as Failed.
                if !matches!(err, JobEngineError::AlreadyExists(_)) {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    match self.job_engine.find_job_by_hydra_id(&session_id).await {
                        Ok(job)
                            if job.status == JobStatus::Pending
                                || job.status == JobStatus::Running =>
                        {
                            warn!(
                                hydra_id = %session_id,
                                create_error = %err,
                                job_status = %job.status,
                                "create_job failed but job exists in K8s; treating as successful"
                            );
                            match self
                                .transition_task_to_pending(&session_id, actor.clone())
                                .await
                            {
                                Ok(_) => {
                                    info!(
                                        hydra_id = %session_id,
                                        "set task status to Pending (job found after create error)"
                                    );
                                }
                                Err(transition_err) => {
                                    warn!(
                                        hydra_id = %session_id,
                                        error = %transition_err,
                                        "failed to set task to Pending after finding existing job"
                                    );
                                }
                            }
                            return;
                        }
                        _ => {
                            // Job not found or in a terminal state — fall through
                            // to the existing failure path below.
                        }
                    }
                }

                let failure_reason = format!("Failed to create Kubernetes job: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &session_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        None,
                        actor,
                    )
                    .await
                {
                    error!(
                        hydra_id = %session_id,
                        error = %update_err,
                        "failed to set task status to Failed (spawn failed)"
                    );
                } else {
                    info!(
                        hydra_id = %session_id,
                        "set task status to Failed (spawn failed)"
                    );
                }
            }
        }
    }

    pub async fn reap_orphaned_jobs(&self) {
        let job_engine_jobs = match self.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                return;
            }
        };

        if job_engine_jobs.is_empty() {
            return;
        }

        let store_session_ids: Vec<SessionId> = {
            let store = self.store.as_ref();
            match store.list_sessions(&SearchSessionsQuery::default()).await {
                Ok(tasks) => tasks.into_iter().map(|(id, _)| id).collect(),
                Err(err) => {
                    error!(error = %err, "failed to list tasks from store for job reconciliation");
                    return;
                }
            }
        };

        let store_task_set: HashSet<_> = store_session_ids.into_iter().collect();
        let orphaned_jobs: Vec<_> = job_engine_jobs
            .into_iter()
            .filter(|job| !store_task_set.contains(&job.id))
            .collect();

        if !orphaned_jobs.is_empty() {
            info!(
                count = orphaned_jobs.len(),
                "killing jobs present in engine but missing from store"
            );
        }

        for job in orphaned_jobs {
            match self.job_engine.kill_job(&job.id).await {
                Ok(()) => {
                    info!(hydra_id = %job.id, "killed job not present in store");
                }
                Err(err) => {
                    warn!(hydra_id = %job.id, error = %err, "failed to kill job not present in store");
                }
            }
        }
    }

    /// Cleans up tasks whose `spawned_from` issue has been soft-deleted.
    ///
    /// For each non-deleted task that references a `spawned_from` issue, checks
    /// whether that issue still exists. If it does not (i.e., it has been
    /// soft-deleted), the task is soft-deleted and any running/pending job is
    /// killed in the engine.
    pub async fn cleanup_orphaned_tasks(&self, actor: ActorRef) {
        let store = self.store.as_ref();
        let tasks = match store.list_sessions(&SearchSessionsQuery::default()).await {
            Ok(tasks) => tasks,
            Err(err) => {
                error!(error = %err, "failed to list tasks for orphaned task cleanup");
                return;
            }
        };

        for (session_id, versioned_task) in tasks {
            let issue_id = match &versioned_task.item.spawned_from {
                Some(id) => id.clone(),
                None => continue,
            };

            let issue_deleted = match store.get_issue(&issue_id, false).await {
                Ok(_) => false,
                Err(StoreError::IssueNotFound(_)) => true,
                Err(err) => {
                    warn!(
                        hydra_id = %session_id,
                        issue_id = %issue_id,
                        error = %err,
                        "failed to check spawned_from issue for orphaned task cleanup"
                    );
                    continue;
                }
            };

            if !issue_deleted {
                continue;
            }

            info!(
                hydra_id = %session_id,
                issue_id = %issue_id,
                "soft-deleting orphaned task whose spawned_from issue was deleted"
            );

            if let Err(err) = self
                .store
                .delete_session_with_actor(&session_id, actor.clone())
                .await
            {
                warn!(
                    hydra_id = %session_id,
                    error = %err,
                    "failed to soft-delete orphaned task"
                );
                continue;
            }

            if matches!(
                versioned_task.item.status,
                Status::Pending | Status::Running
            ) {
                if let Err(err) = self.job_engine.kill_job(&session_id).await {
                    warn!(
                        hydra_id = %session_id,
                        error = %err,
                        "failed to kill job for orphaned task"
                    );
                }
            }
        }
    }

    pub async fn reconcile_running_task(&self, session_id: SessionId, actor: ActorRef) {
        let current_status = {
            let store = self.store.as_ref();
            match store.get_session(&session_id, false).await {
                Ok(versioned) => versioned.item.status,
                Err(err) => {
                    warn!(
                        hydra_id = %session_id,
                        error = %err,
                        "failed to load task while reconciling status"
                    );
                    return;
                }
            }
        };

        match self.job_engine.find_job_by_hydra_id(&session_id).await {
            Ok(job) => match job.status {
                JobStatus::Pending => {}
                JobStatus::Running => {
                    if current_status == Status::Pending {
                        match self
                            .transition_task_to_running(&session_id, actor.clone())
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    hydra_id = %session_id,
                                    "set task status to Running (pod started)"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    hydra_id = %session_id,
                                    error = %err,
                                    "failed to set task to Running after pod start"
                                );
                            }
                        }
                    }
                }
                JobStatus::Complete => {
                    warn!(
                        hydra_id = %session_id,
                        "Job completed in job engine without submitting results."
                    );

                    let completion_time = job.completion_time.unwrap_or_else(Utc::now);
                    let duration_since_completion =
                        Utc::now().signed_duration_since(completion_time);

                    if duration_since_completion < Duration::seconds(60) {
                        return;
                    }

                    let failure_reason =
                        "Job completed without submitting results (timeout after 1 minute)"
                            .to_string();
                    match self
                        .transition_task_to_completion(
                            &session_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            None,
                            actor.clone(),
                        )
                        .await
                    {
                        Ok(_) => {
                            warn!(hydra_id = %session_id, "task marked failed due to missing results after job completion timeout");
                        }
                        Err(err) => {
                            warn!(hydra_id = %session_id, error = %err, "failed to mark task failed after missing results timeout");
                        }
                    }
                }
                JobStatus::Failed => {
                    let failure_reason = job
                        .failure_message
                        .unwrap_or_else(|| "Job failed for an undetermined reason".to_string());
                    match self
                        .transition_task_to_completion(
                            &session_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            None,
                            actor.clone(),
                        )
                        .await
                    {
                        Ok(_) => {
                            info!(hydra_id = %session_id, "updated task status to Failed from job engine");
                        }
                        Err(err) => {
                            warn!(hydra_id = %session_id, error = %err, "failed to update task status to Failed");
                        }
                    }
                }
            },
            Err(JobEngineError::NotFound(_)) => {
                warn!(
                    hydra_id = %session_id,
                    "job not found in job engine, marking as failed"
                );

                let failure_reason = "Job not found in job engine".to_string();
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &session_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        None,
                        actor,
                    )
                    .await
                {
                    error!(hydra_id = %session_id, error = %update_err, "failed to set task status to Failed");
                }
            }
            Err(err) => {
                error!(
                    hydra_id = %session_id,
                    error = %err,
                    "failed to check job status in job engine"
                );
            }
        }
    }

    pub async fn transition_task_to_pending(
        &self,
        session_id: &SessionId,
        actor: ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let latest = self.store.get_session(session_id, false).await?;
        if latest.item.status != Status::Created {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Pending;
        updated.last_message = None;
        updated.error = None;

        self.store
            .update_session_with_actor(session_id, updated, actor)
            .await
    }

    pub async fn transition_task_to_running(
        &self,
        session_id: &SessionId,
        actor: ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let latest = self.store.get_session(session_id, false).await?;
        if !matches!(latest.item.status, Status::Created | Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Running;
        updated.last_message = None;
        updated.error = None;
        if updated.start_time.is_none() {
            updated.start_time = Some(Utc::now());
        }

        self.store
            .update_session_with_actor(session_id, updated, actor)
            .await
    }

    pub async fn transition_task_to_completion(
        &self,
        session_id: &SessionId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        usage: Option<hydra_common::sessions::TokenUsage>,
        actor: ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let store = self.store.as_ref();
        let latest = store.get_session(session_id, false).await?;
        let can_transition = match latest.item.status {
            Status::Created => result.is_err(),
            Status::Pending | Status::Running => true,
            // Idempotent: if already in the target terminal state, return Ok
            Status::Complete => result.is_ok(),
            Status::Failed => result.is_err(),
        };
        if !can_transition {
            return Err(StoreError::InvalidStatusTransition);
        }

        // Already in the target terminal state — return existing version unchanged
        if latest.item.status.is_terminal() {
            return Ok(latest);
        }

        let mut updated = latest.item;
        match result {
            Ok(()) => {
                updated.status = Status::Complete;
                updated.last_message = last_message;
                updated.error = None;
                updated.usage = usage;
            }
            Err(error) => {
                updated.status = Status::Failed;
                updated.last_message = None;
                updated.error = Some(error);
            }
        }
        if updated.end_time.is_none() {
            updated.end_time = Some(Utc::now());
        }

        self.store
            .update_session_with_actor(session_id, updated, actor)
            .await
    }

    /// Upsert a proxy target (port + optional ready_path) on a session.
    /// Idempotent: re-posting an existing `port` replaces `ready_path`.
    pub async fn upsert_proxy_target(
        &self,
        session_id: &SessionId,
        target: hydra_common::api::v1::sessions::ProxyTarget,
        actor: ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let latest = self.store.get_session(session_id, false).await?;
        let mut updated = latest.item;
        match updated
            .proxy_targets
            .iter_mut()
            .find(|t| t.port == target.port)
        {
            Some(existing) => *existing = target,
            None => updated.proxy_targets.push(target),
        }
        self.store
            .update_session_with_actor(session_id, updated, actor)
            .await
    }

    /// Remove a proxy target by `port`. Idempotent: returns the current
    /// `Versioned<Session>` unchanged when the port is absent.
    pub async fn remove_proxy_target(
        &self,
        session_id: &SessionId,
        port: u16,
        actor: ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let latest = self.store.get_session(session_id, false).await?;
        let mut updated = latest.item;
        let before = updated.proxy_targets.len();
        updated.proxy_targets.retain(|t| t.port != port);
        if updated.proxy_targets.len() == before {
            return Ok(Versioned {
                item: updated,
                version: latest.version,
                timestamp: latest.timestamp,
                actor: latest.actor,
                creation_time: latest.creation_time,
            });
        }
        self.store
            .update_session_with_actor(session_id, updated, actor)
            .await
    }

    pub(crate) async fn get_latest_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Session, StoreError> {
        self.get_session(session_id).await
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Result<Session, StoreError> {
        let store = self.store.as_ref();
        store.get_session(session_id, false).await.map(|v| v.item)
    }

    pub async fn get_session_versions(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError> {
        let store = self.store.as_ref();
        store.get_session_versions(session_id).await
    }

    pub async fn get_sessions_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<SessionId>, StoreError> {
        let store = self.store.as_ref();
        store.get_sessions_for_issue(issue_id).await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionId>, StoreError> {
        let store = self.store.as_ref();
        store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .map(|tasks| tasks.into_iter().map(|(id, _)| id).collect())
    }

    pub async fn list_sessions_with_query(
        &self,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_sessions(query).await
    }

    pub async fn count_sessions(&self, query: &SearchSessionsQuery) -> Result<u64, StoreError> {
        let store = self.store.as_ref();
        store.count_sessions(query).await
    }

    pub async fn get_status_log(
        &self,
        session_id: &SessionId,
    ) -> Result<TaskStatusLog, StoreError> {
        let store = self.store.as_ref();
        store.get_status_log(session_id).await
    }

    pub async fn get_status_logs(
        &self,
        session_ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError> {
        let store = self.store.as_ref();
        store.get_status_logs(session_ids).await
    }
}

/// Container-side mount point prefix for local repos.
const CONTAINER_REPO_MOUNT_PREFIX: &str = "/mnt/repos";

/// Inspects the resolved bundle for a `file://` URL and, if found, constructs a
/// bind mount mapping the host path into the container. The bundle URL is
/// rewritten in-place to the container-side `file://` path so that all
/// downstream consumers (including the worker context endpoint) automatically
/// receive the correct URL.
///
/// Returns an empty `Vec` when the bundle is not a local `file://` repo.
pub(crate) fn build_bind_mounts_for_local_repo(bundle: &mut Bundle) -> Vec<BindMount> {
    let (host_path, container_path) = match rewrite_local_bundle_url(bundle) {
        Some(paths) => paths,
        None => return Vec::new(),
    };

    info!(
        host_path = host_path,
        container_path = container_path,
        "mounting local repo into container"
    );

    vec![BindMount {
        host_path,
        container_path,
    }]
}

/// Walks a [`MountSpec`]'s mounts and rewrites every `MountItem::Bundle` with
/// a `file://` URL to point at a container-side mount path. Returns the
/// resulting bind mounts so the caller can pass them to the container engine.
///
/// Used by [`crate::routes::sessions::context::get_session_context`] to serve
/// container-side URLs to the worker when the engine is containerized.
pub(crate) fn rewrite_local_bundle_urls(mount_spec: &mut MountSpec) -> Vec<BindMount> {
    let mut bind_mounts = Vec::new();
    for mount in mount_spec.mounts.iter_mut() {
        if let MountItem::Bundle { bundle, .. } = mount {
            bind_mounts.extend(build_bind_mounts_for_local_repo(bundle));
        }
    }
    bind_mounts
}

/// If `bundle` is a `GitRepository` with a `file://` URL, rewrites the URL to
/// a container-side mount path and returns `(host_path, container_path)`.
/// Returns `None` when no rewriting is needed.
pub(crate) fn rewrite_local_bundle_url(bundle: &mut Bundle) -> Option<(String, String)> {
    let url = match bundle {
        &mut Bundle::GitRepository { ref url, .. } => url.clone(),
        _ => return None,
    };

    let host_path = match url.strip_prefix("file://") {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => return None,
    };

    // Derive a stable mount name from the last path component.
    let repo_name = std::path::Path::new(&host_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");

    let container_path = format!("{CONTAINER_REPO_MOUNT_PREFIX}/{repo_name}");
    let container_url = format!("file://{container_path}");

    // Rewrite the bundle URL in-place.
    if let Bundle::GitRepository { url, .. } = bundle {
        *url = container_url;
    }

    Some((host_path, container_path))
}

#[cfg(test)]
mod tests {
    use crate::{
        app::test_helpers::{
            issue_with_status, sample_task, state_with_default_model, task_for_issue,
        },
        domain::actors::ActorRef,
        domain::issues::{Issue, IssueStatus, IssueType, SessionSettings},
        domain::users::Username,
        job_engine::{JobEngine, JobStatus},
        store::{ReadOnlyStore, Status, StoreError, TaskError},
        test_utils::{MockJobEngine, test_state_with_engine},
    };
    use chrono::{Duration, Utc};
    use hydra_common::SessionId;
    use std::sync::Arc;

    #[tokio::test]
    async fn start_pending_task_spawns_and_marks_pending() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let config = state.config.clone();
        let session = sample_task();

        let (session_id, _) = {
            let store = state.store.as_ref();
            store
                .add_session_with_actor(session, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        state
            .start_pending_task(session_id.clone(), ActorRef::test())
            .await;

        {
            let store = state.store.as_ref();
            let status = store
                .get_session(&session_id, false)
                .await
                .unwrap()
                .item
                .status;
            assert_eq!(status, Status::Pending);
        }

        assert!(job_engine.env_vars_for_job(&session_id).is_some());
        let limits = job_engine
            .resource_limits_for_job(&session_id)
            .expect("resource limits should be recorded");
        assert_eq!(
            limits,
            (
                config.job.cpu_limit.clone(),
                config.job.memory_limit.clone()
            )
        );
    }

    #[tokio::test]
    async fn start_pending_task_uses_task_resource_limits() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let session_settings = SessionSettings {
            cpu_limit: Some("750m".to_string()),
            memory_limit: Some("2Gi".to_string()),
            ..Default::default()
        };

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    Issue {
                        issue_type: IssueType::Task,
                        title: String::new(),
                        description: "with limits".to_string(),
                        creator: Username::from("creator"),
                        progress: String::new(),
                        status: IssueStatus::Open.into(),
                        project_id: None,
                        assignee: None,
                        session_settings: session_settings.clone(),
                        dependencies: Vec::new(),
                        patches: Vec::new(),
                        deleted: false,
                        form: None,
                        form_response: None,
                        feedback: None,
                    },
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        let (session_id, _) = {
            let store = state.store.as_ref();
            let mut session = task_for_issue(&issue_id);
            session.cpu_limit = session_settings.cpu_limit.clone();
            session.memory_limit = session_settings.memory_limit.clone();
            store
                .add_session_with_actor(session, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        state
            .start_pending_task(session_id.clone(), ActorRef::test())
            .await;

        let limits = job_engine
            .resource_limits_for_job(&session_id)
            .expect("resource limits should be recorded");
        assert_eq!(limits, ("750m".to_string(), "2Gi".to_string()));
    }

    #[tokio::test]
    async fn start_pending_task_timeout_but_job_exists_transitions_to_pending() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let session = sample_task();

        let (session_id, _) = {
            let store = state.store.as_ref();
            store
                .add_session_with_actor(session, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        // Pre-insert the job so find_job_by_hydra_id finds it, and configure
        // create_job to fail (simulating an etcdserver timeout where the job
        // was actually created).
        job_engine.insert_job(&session_id, JobStatus::Running).await;
        job_engine.set_create_job_error(Some("etcdserver: request timed out".to_string()));

        state
            .start_pending_task(session_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        let status = store
            .get_session(&session_id, false)
            .await
            .unwrap()
            .item
            .status;
        assert_eq!(status, Status::Pending);
    }

    #[tokio::test]
    async fn start_pending_task_timeout_and_job_missing_transitions_to_failed() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let session = sample_task();

        let (session_id, _) = {
            let store = state.store.as_ref();
            store
                .add_session_with_actor(session, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        // Configure create_job to fail without pre-inserting the job, so
        // find_job_by_hydra_id will return NotFound.
        job_engine.set_create_job_error(Some("etcdserver: request timed out".to_string()));

        state
            .start_pending_task(session_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        let status = store
            .get_session(&session_id, false)
            .await
            .unwrap()
            .item
            .status;
        assert_eq!(status, Status::Failed);
    }

    /// Regression: i-lkadrfky. Sessions whose request mount_spec is empty
    /// and whose session_settings carry no repo (chat conversations,
    /// PM/breakdown on a repo-less parent issue) must be persisted with a
    /// `Bundle::None`-bearing MountSpec so the worker's `BundleMount`
    /// materializes `working_dir` before `Command::current_dir(working_dir)`.
    /// A spec with `mounts: []` would otherwise hand the worker a
    /// non-existent `dest/repo` and the spawn would ENOENT.
    #[tokio::test]
    async fn create_session_coerces_empty_mount_spec_to_bundle_none() {
        use crate::domain::conversations::{Conversation, ConversationStatus};
        use hydra_common::api::v1::sessions::{
            AgentSpec, Bundle, CreateSessionRequest, MountItem, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");

        // Headless session with no spawned_from — the only inputs are an
        // empty `MountSpec::default()` and no session_settings to derive
        // from. Pre-fix this produced an empty `mounts: []`.
        let headless_request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Adhoc {
                system_prompt: "test".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };
        let (headless_id, _) = state
            .create_session(
                headless_request,
                ActorRef::test(),
                Username::from("creator"),
            )
            .await
            .unwrap();
        let headless = state.get_session(&headless_id).await.unwrap();
        assert!(
            !headless.mount_spec.is_empty(),
            "headless session without repo must still carry a Bundle mount to create working_dir"
        );
        assert!(
            matches!(
                headless.mount_spec.mounts.first(),
                Some(MountItem::Bundle {
                    bundle: Bundle::None,
                    ..
                })
            ),
            "expected first mount to be a Bundle::None; got {:?}",
            headless.mount_spec.mounts
        );

        // Interactive (chat) session whose conversation has no
        // session_settings repo info — the path that the e2e tester
        // confirmed reproduces the ENOENT spawn failure.
        let (conv_id, _) = state
            .store
            .add_conversation_with_actor(
                Conversation {
                    title: None,
                    agent_name: None,
                    status: ConversationStatus::Active,
                    creator: Username::from("creator"),
                    session_settings: crate::domain::issues::SessionSettings::default(),
                    spawned_from: None,
                    deleted: false,
                },
                ActorRef::test(),
            )
            .await
            .unwrap();
        let chat_request = CreateSessionRequest {
            mode: SessionMode::Interactive {
                conversation_id: conv_id,
                idle_timeout_secs: None,
                greet_user: false,
            },
            agent_config: AgentSpec::Adhoc {
                system_prompt: "test".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };
        let (chat_id, _) = state
            .create_session(chat_request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let chat = state.get_session(&chat_id).await.unwrap();
        assert!(
            !chat.mount_spec.is_empty(),
            "chat session must still carry a Bundle mount to create working_dir"
        );
        assert!(
            matches!(
                chat.mount_spec.mounts.first(),
                Some(MountItem::Bundle {
                    bundle: Bundle::None,
                    ..
                })
            ),
            "expected first mount to be a Bundle::None; got {:?}",
            chat.mount_spec.mounts
        );
    }

    #[tokio::test]
    async fn create_session_passes_interactive_and_conversation_id() {
        use crate::domain::conversations::{Conversation, ConversationStatus};
        use hydra_common::api::v1::sessions::{
            AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");

        // Headless session — no conversation lookup, no spawned_from.
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Adhoc {
                system_prompt: "test".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        assert!(
            !session.is_interactive(),
            "headless session should not be interactive"
        );
        assert_eq!(
            session.conversation_id(),
            None,
            "headless session should have no conversation_id"
        );

        // Interactive session — conversation_id lives on the SessionMode.
        let (conv_id, _) = state
            .store
            .add_conversation_with_actor(
                Conversation {
                    title: None,
                    agent_name: None,
                    status: ConversationStatus::Active,
                    creator: Username::from("creator"),
                    session_settings: crate::domain::issues::SessionSettings::default(),
                    spawned_from: None,
                    deleted: false,
                },
                ActorRef::test(),
            )
            .await
            .unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Interactive {
                conversation_id: conv_id.clone(),
                idle_timeout_secs: None,
                greet_user: false,
            },
            agent_config: AgentSpec::Adhoc {
                system_prompt: "test".to_string(),
                mcp_config: None,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        assert!(
            session.is_interactive(),
            "conversation session should be interactive"
        );
        assert_eq!(
            session.conversation_id().cloned(),
            Some(conv_id),
            "conversation session should have conversation_id"
        );
    }

    /// Issues with `project_id = None` should exercise the four-level
    /// prompt resolver via the DefaultProject path references. Because
    /// the new system / project / status docs don't yet exist in the
    /// doc store (PR 2 authors them), all three new layers resolve to
    /// empty slices and the spawned session's `system_prompt` is
    /// byte-identical to today's agent prompt — that's the "no
    /// observable behavior change" invariant from the design.
    #[tokio::test]
    async fn create_session_for_issue_with_no_project_id_returns_agent_body_only() {
        use crate::domain::agents::Agent;
        use crate::domain::documents::Document;
        use hydra_common::api::v1::sessions::{
            AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");

        let agent_name = "swe";
        let prompt_path = format!("/agents/{agent_name}/prompt.md");
        let agent = Agent::new(
            agent_name.to_string(),
            prompt_path.clone(),
            None,
            1,
            1,
            false,
            false,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();
        let doc = Document {
            title: "swe".to_string(),
            body_markdown: "AGENT BODY".to_string(),
            path: Some(prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();

        let issue = issue_with_status("test", IssueStatus::Open, vec![]);
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        let agent_name_typed =
            hydra_common::api::v1::agents::AgentName::try_new(agent_name).unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Named {
                name: agent_name_typed,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: Some(issue_id),
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        assert_eq!(session.resolved_prompt(), "AGENT BODY");
    }

    /// Conversation sessions (no `spawned_from`) must use the no-project
    /// sentinel, so the four-level resolver emits system + agent only.
    /// In PR 1 the system doc also doesn't exist, so the effective output
    /// equals just the agent body.
    #[tokio::test]
    async fn create_session_without_spawned_from_uses_no_project_sentinel() {
        use crate::domain::agents::Agent;
        use crate::domain::documents::Document;
        use hydra_common::api::v1::sessions::{
            AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");
        let agent_name = "chat";
        let prompt_path = format!("/agents/{agent_name}/prompt.md");
        let agent = Agent::new(
            agent_name.to_string(),
            prompt_path.clone(),
            None,
            1,
            1,
            false,
            false,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();
        let doc = Document {
            title: "chat".to_string(),
            body_markdown: "CHAT AGENT BODY".to_string(),
            path: Some(prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();

        let agent_name_typed =
            hydra_common::api::v1::agents::AgentName::try_new(agent_name).unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Named {
                name: agent_name_typed,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: None,
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        assert_eq!(session.resolved_prompt(), "CHAT AGENT BODY");
    }

    /// Regression test for [[i-voxdzsyb]]. A session spawned on an issue
    /// whose `project_id` points at an engineering-v2-style project must
    /// concatenate the project + status prompt slices onto
    /// `agent_config.system_prompt`. Prior to the
    /// `add_projects_prompt_path` migration the project-level
    /// `prompt_path` was silently dropped by the store, so the spawned
    /// session saw only the agent slice.
    #[tokio::test]
    async fn slice_prompts_reach_engineering_v2_sessions_backlog() {
        use crate::domain::agents::Agent;
        use crate::domain::documents::Document;
        use hydra_common::api::v1::projects::{
            Project as ApiProject, ProjectKey, StatusDefinition, StatusKey,
        };
        use hydra_common::api::v1::sessions::{
            AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");

        // ---- Seed the agent + its prompt doc.
        let agent_name = "pm";
        let agent_prompt_path = format!("/agents/{agent_name}/prompt.md");
        let agent = Agent::new(
            agent_name.to_string(),
            agent_prompt_path.clone(),
            None,
            1,
            1,
            false,
            false,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();
        let agent_doc = Document {
            title: agent_name.to_string(),
            body_markdown: "PM AGENT BODY".to_string(),
            path: Some(agent_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(agent_doc, ActorRef::test())
            .await
            .unwrap();

        // ---- Seed the engineering-v2-style project. Both the project
        // and the `backlog` status declare a doc-store `prompt_path`.
        let project_prompt_path = "/projects/engineering-v2/prompt.md";
        let backlog_prompt_path = "/projects/engineering-v2/statuses/backlog.md";
        let backlog_status = {
            let mut def = StatusDefinition::new(
                StatusKey::try_new("backlog").unwrap(),
                "Backlog".to_string(),
                "#9b59b6".parse().unwrap(),
                false,
                false,
                false,
                None,
            );
            def.prompt_path = Some(backlog_prompt_path.to_string());
            def
        };
        let mut project = ApiProject::new(
            ProjectKey::try_new("engineering-v2").unwrap(),
            "Engineering v2".to_string(),
            vec![backlog_status],
            StatusKey::try_new("backlog").unwrap(),
            hydra_common::api::v1::users::Username::from("alice"),
            false,
        );
        project.prompt_path = Some(project_prompt_path.to_string());
        let (project_id, _) = state
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();

        // ---- Seed the project + status prompt docs.
        let project_doc = Document {
            title: "engineering-v2 project prompt".to_string(),
            body_markdown: "PROJECT SLICE — engineering-v2".to_string(),
            path: Some(project_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(project_doc, ActorRef::test())
            .await
            .unwrap();
        let backlog_doc = Document {
            title: "engineering-v2 backlog prompt".to_string(),
            body_markdown: "STATUS SLICE — backlog (engineering-v2)".to_string(),
            path: Some(backlog_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(backlog_doc, ActorRef::test())
            .await
            .unwrap();

        // ---- Seed the issue, bound to the project at the `backlog` status.
        let mut issue = issue_with_status("v2 backlog issue", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = StatusKey::try_new("backlog").unwrap();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        // ---- Spawn the session and read back the resolved system prompt.
        let agent_name_typed =
            hydra_common::api::v1::agents::AgentName::try_new(agent_name).unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Named {
                name: agent_name_typed,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: Some(issue_id),
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        let prompt = session.resolved_prompt();
        assert!(
            prompt.contains("PM AGENT BODY"),
            "expected agent slice in system_prompt; got {prompt:?}"
        );
        assert!(
            prompt.contains("PROJECT SLICE — engineering-v2"),
            "expected project slice in system_prompt; got {prompt:?}"
        );
        assert!(
            prompt.contains("STATUS SLICE — backlog (engineering-v2)"),
            "expected backlog status slice in system_prompt; got {prompt:?}"
        );
    }

    /// Same as [`slice_prompts_reach_engineering_v2_sessions_backlog`] but
    /// with an issue at the `in-review` status. Confirms the resolver
    /// picks the right status slice when the issue's status changes —
    /// guards against a regression where the resolver hard-codes a
    /// status key.
    #[tokio::test]
    async fn slice_prompts_reach_engineering_v2_sessions_in_review() {
        use crate::domain::agents::Agent;
        use crate::domain::documents::Document;
        use hydra_common::api::v1::projects::{
            Project as ApiProject, ProjectKey, StatusDefinition, StatusKey,
        };
        use hydra_common::api::v1::sessions::{
            AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
        };

        let state = state_with_default_model("default-model");
        let agent_name = "reviewer";
        let agent_prompt_path = format!("/agents/{agent_name}/prompt.md");
        let agent = Agent::new(
            agent_name.to_string(),
            agent_prompt_path.clone(),
            None,
            1,
            1,
            false,
            false,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();
        let agent_doc = Document {
            title: agent_name.to_string(),
            body_markdown: "REVIEWER AGENT BODY".to_string(),
            path: Some(agent_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(agent_doc, ActorRef::test())
            .await
            .unwrap();

        let project_prompt_path = "/projects/engineering-v2/prompt.md";
        let in_review_prompt_path = "/projects/engineering-v2/statuses/in-review.md";
        let in_review_status = {
            let mut def = StatusDefinition::new(
                StatusKey::try_new("in-review").unwrap(),
                "In review".to_string(),
                "#f1c40f".parse().unwrap(),
                false,
                false,
                false,
                None,
            );
            def.prompt_path = Some(in_review_prompt_path.to_string());
            def
        };
        let mut project = ApiProject::new(
            ProjectKey::try_new("engineering-v2").unwrap(),
            "Engineering v2".to_string(),
            vec![in_review_status],
            StatusKey::try_new("in-review").unwrap(),
            hydra_common::api::v1::users::Username::from("alice"),
            false,
        );
        project.prompt_path = Some(project_prompt_path.to_string());
        let (project_id, _) = state
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();

        let project_doc = Document {
            title: "engineering-v2 project prompt".to_string(),
            body_markdown: "PROJECT SLICE — engineering-v2".to_string(),
            path: Some(project_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(project_doc, ActorRef::test())
            .await
            .unwrap();
        let in_review_doc = Document {
            title: "engineering-v2 in-review prompt".to_string(),
            body_markdown: "STATUS SLICE — same-issue review hand-off".to_string(),
            path: Some(in_review_prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(in_review_doc, ActorRef::test())
            .await
            .unwrap();

        let mut issue = issue_with_status("v2 in-review issue", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = StatusKey::try_new("in-review").unwrap();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        let agent_name_typed =
            hydra_common::api::v1::agents::AgentName::try_new(agent_name).unwrap();
        let request = CreateSessionRequest {
            mode: SessionMode::Headless,
            agent_config: AgentSpec::Named {
                name: agent_name_typed,
            },
            model: None,
            mount_spec: MountSpec::default(),
            image: None,
            env_vars: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            spawned_from: Some(issue_id),
            resumed_from: None,
        };
        let (session_id, _) = state
            .create_session(request, ActorRef::test(), Username::from("creator"))
            .await
            .unwrap();
        let session = state.get_session(&session_id).await.unwrap();
        let prompt = session.resolved_prompt();
        assert!(
            prompt.contains("REVIEWER AGENT BODY"),
            "expected agent slice in system_prompt; got {prompt:?}"
        );
        assert!(
            prompt.contains("PROJECT SLICE — engineering-v2"),
            "expected project slice in system_prompt; got {prompt:?}"
        );
        assert!(
            prompt.contains("same-issue review hand-off"),
            "expected in-review status slice in system_prompt; got {prompt:?}"
        );
    }

    #[test]
    fn apply_session_settings_defaults_sets_model() {
        let state = state_with_default_model("gpt-4o");
        let session_settings = SessionSettings::default();

        let resolved = state.apply_session_settings_defaults(session_settings);

        assert_eq!(resolved.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn apply_session_settings_defaults_preserves_explicit_model() {
        let state = state_with_default_model("gpt-4o");
        let session_settings = SessionSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let resolved = state.apply_session_settings_defaults(session_settings);

        assert_eq!(resolved.model.as_deref(), Some("custom-model"));
    }

    #[tokio::test]
    async fn reap_orphaned_jobs_kills_jobs_missing_from_store() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let (tracked_session_id, _) = {
            let store = state.store.as_ref();
            store
                .add_session_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };
        let orphan_session_id = SessionId::new();

        job_engine
            .insert_job(&tracked_session_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&orphan_session_id, JobStatus::Running)
            .await;

        state.reap_orphaned_jobs().await;

        let tracked_status = job_engine
            .find_job_by_hydra_id(&tracked_session_id)
            .await
            .expect("tracked job should exist")
            .status;
        assert_eq!(tracked_status, JobStatus::Running);

        let orphan_status = job_engine
            .find_job_by_hydra_id(&orphan_session_id)
            .await
            .expect("orphan job should exist")
            .status;
        assert_eq!(orphan_status, JobStatus::Failed);
    }

    #[tokio::test]
    async fn reconcile_running_task_marks_missing_jobs_failed() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let session_id = {
            let store = state.store.as_ref();
            let (session_id, _) = store
                .add_session_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            state
                .transition_task_to_pending(&session_id, ActorRef::test())
                .await
                .expect("task should transition to pending");
            session_id
        };

        state
            .reconcile_running_task(session_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        assert_eq!(
            store
                .get_session(&session_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Failed
        );

        let status_log = store.get_status_log(&session_id).await.unwrap();
        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => {
                assert_eq!(reason, "Job not found in job engine");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reconcile_running_task_times_out_completed_jobs_without_results() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let completion_time = Utc::now() - Duration::seconds(90);

        let session_id = {
            let store = state.store.as_ref();
            let (session_id, _) = store
                .add_session_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            state
                .transition_task_to_pending(&session_id, ActorRef::test())
                .await
                .expect("task should transition to pending");
            session_id
        };

        job_engine
            .insert_job_with_metadata(
                &session_id,
                JobStatus::Complete,
                Some(completion_time),
                None,
            )
            .await;

        state
            .reconcile_running_task(session_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        assert_eq!(
            store
                .get_session(&session_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Failed
        );
        let status_log = store.get_status_log(&session_id).await.unwrap();
        assert!(status_log.end_time().is_some());

        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => assert_eq!(
                reason,
                "Job completed without submitting results (timeout after 1 minute)"
            ),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_deletes_task_with_deleted_issue() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (session_id, _) = store
            .add_session_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let result = store.get_session(&session_id, false).await;
        assert!(
            matches!(result, Err(StoreError::SessionNotFound(_))),
            "orphaned task should be soft-deleted"
        );

        let deleted_task = store.get_session(&session_id, true).await.unwrap();
        assert!(deleted_task.item.deleted);
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_leaves_task_with_existing_issue() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (session_id, _) = store
            .add_session_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let session = store.get_session(&session_id, false).await.unwrap();
        assert!(
            !session.item.deleted,
            "task with existing issue should not be deleted"
        );
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_leaves_task_with_no_spawned_from() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (session_id, _) = store
            .add_session_with_actor(sample_task(), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let session = store.get_session(&session_id, false).await.unwrap();
        assert!(
            !session.item.deleted,
            "task without spawned_from should not be deleted"
        );
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_kills_running_job() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (session_id, _) = store
            .add_session_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&session_id, ActorRef::test())
            .await
            .expect("task should transition to pending");

        job_engine.insert_job(&session_id, JobStatus::Running).await;

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let result = store.get_session(&session_id, false).await;
        assert!(
            matches!(result, Err(StoreError::SessionNotFound(_))),
            "orphaned running task should be soft-deleted"
        );

        let job = job_engine
            .find_job_by_hydra_id(&session_id)
            .await
            .expect("job should still exist in engine");
        assert_eq!(
            job.status,
            JobStatus::Failed,
            "running job for orphaned task should be killed"
        );
    }

    mod bind_mount_tests {
        use super::super::*;
        use hydra_common::api::v1::sessions::Bundle;

        #[test]
        fn file_url_creates_bind_mount_and_rewrites_bundle() {
            let mut bundle = Bundle::GitRepository {
                url: "file:///home/user/my-repo.git".to_string(),
                rev: "main".to_string(),
            };

            let mounts = build_bind_mounts_for_local_repo(&mut bundle);

            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].host_path, "/home/user/my-repo.git");
            assert_eq!(mounts[0].container_path, "/mnt/repos/my-repo.git");
            match &bundle {
                Bundle::GitRepository { url, .. } => {
                    assert_eq!(url, "file:///mnt/repos/my-repo.git");
                }
                _ => panic!("expected GitRepository bundle"),
            }
        }

        #[test]
        fn https_url_returns_no_bind_mounts() {
            let mut bundle = Bundle::GitRepository {
                url: "https://github.com/owner/repo.git".to_string(),
                rev: "main".to_string(),
            };

            let mounts = build_bind_mounts_for_local_repo(&mut bundle);

            assert!(mounts.is_empty());
            // URL should be unchanged.
            match &bundle {
                Bundle::GitRepository { url, .. } => {
                    assert_eq!(url, "https://github.com/owner/repo.git");
                }
                _ => panic!("expected GitRepository bundle"),
            }
        }

        #[test]
        fn none_bundle_returns_no_bind_mounts() {
            let mut bundle = Bundle::None;

            let mounts = build_bind_mounts_for_local_repo(&mut bundle);

            assert!(mounts.is_empty());
        }

        #[test]
        fn file_url_with_nested_path() {
            let mut bundle = Bundle::GitRepository {
                url: "file:///srv/git/projects/my-project".to_string(),
                rev: "develop".to_string(),
            };

            let mounts = build_bind_mounts_for_local_repo(&mut bundle);

            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].host_path, "/srv/git/projects/my-project");
            assert_eq!(mounts[0].container_path, "/mnt/repos/my-project");
            match &bundle {
                Bundle::GitRepository { url, .. } => {
                    assert_eq!(url, "file:///mnt/repos/my-project");
                }
                _ => panic!("expected GitRepository bundle"),
            }
        }

        /// PR-F: `routes/sessions/context.rs::get_session_context` walks
        /// `session.mount_spec.mounts` via [`rewrite_local_bundle_urls`] and
        /// only rewrites Bundle items whose `Bundle::GitRepository.url` is a
        /// `file://` URL. Non-`file://` URLs and `Bundle::None` items are
        /// left alone.
        #[test]
        fn rewrite_local_bundle_urls_walks_mount_spec_and_skips_non_file_urls() {
            use hydra_common::api::v1::sessions::{MountItem, MountSpec, RelativePath};

            let mut spec = MountSpec::new(
                RelativePath::new("repo").unwrap(),
                vec![
                    MountItem::Bundle {
                        target: RelativePath::new("repo").unwrap(),
                        bundle: Bundle::GitRepository {
                            url: "file:///home/user/local-repo.git".to_string(),
                            rev: "main".to_string(),
                        },
                    },
                    MountItem::Bundle {
                        target: RelativePath::new("vendored").unwrap(),
                        bundle: Bundle::GitRepository {
                            url: "https://github.com/owner/repo.git".to_string(),
                            rev: "main".to_string(),
                        },
                    },
                    MountItem::Documents {
                        target: RelativePath::new("documents").unwrap(),
                    },
                ],
            );

            let bind_mounts = rewrite_local_bundle_urls(&mut spec);

            assert_eq!(bind_mounts.len(), 1, "only the file:// URL should bind");
            assert_eq!(bind_mounts[0].host_path, "/home/user/local-repo.git");
            assert_eq!(bind_mounts[0].container_path, "/mnt/repos/local-repo.git");

            match &spec.mounts[0] {
                MountItem::Bundle {
                    bundle: Bundle::GitRepository { url, .. },
                    ..
                } => {
                    assert_eq!(url, "file:///mnt/repos/local-repo.git");
                }
                other => panic!("expected first item to remain a Bundle, got {other:?}"),
            }
            match &spec.mounts[1] {
                MountItem::Bundle {
                    bundle: Bundle::GitRepository { url, .. },
                    ..
                } => {
                    assert_eq!(
                        url, "https://github.com/owner/repo.git",
                        "non-file:// URLs must not be rewritten"
                    );
                }
                other => panic!("expected second item to remain a Bundle, got {other:?}"),
            }
        }
    }
}
