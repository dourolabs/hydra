#[cfg(test)]
use crate::domain::issues::{IssueDependency, IssueType};
#[cfg(test)]
use crate::domain::users::Username;
#[cfg(test)]
use crate::store::Session;
use crate::{
    app::AppState,
    domain::{
        actors::ActorRef,
        conversations::ConversationStatus,
        issues::{Issue, IssueDependencyType},
    },
    store::{ReadOnlyStore, Status, StoreError},
};
use anyhow::Context;
#[cfg(test)]
use hydra_common::RepoName;
use hydra_common::api::v1 as api;
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::projects::StatusKey;
use hydra_common::api::v1::sessions::SearchSessionsQuery;
#[cfg(test)]
use hydra_common::test_utils::status::status;
use hydra_common::{ConversationId, IssueId, SessionId, VersionNumber};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared spawn attempt state that persists across scheduler iterations.
pub type SharedSpawnAttempts = Arc<RwLock<HashMap<IssueId, SpawnAttempt>>>;

pub const ISSUE_ID_ENV_VAR: &str = "HYDRA_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "HYDRA_AGENT_NAME";

/// Result of attempting to spawn a session for an issue.
pub(crate) enum SpawnResult {
    /// A session was created in the store. The id is returned so the outer
    /// automation loop can log it / decrement remaining capacity / etc.
    /// (PR-E §2.5: `build_task` no longer just builds a `Session` — it goes
    /// through `AppState::create_session`, which persists the row.)
    Spawned(SessionId),
    /// A conversation was created in the store for an interactive-status
    /// issue. The companion session is materialized asynchronously by
    /// `SpawnConversationSessionsAutomation`.
    SpawnedConversation(ConversationId),
    /// The issue was skipped (not eligible for spawning).
    Skipped,
    /// The spawn attempt retry cap has been exhausted for this issue.
    RetriesExhausted { issue_id: IssueId },
}

#[cfg(test)]
impl SpawnResult {
    /// Returns the spawned session id, or `None` if the result is not `Spawned`.
    fn into_session_id(self) -> Option<SessionId> {
        match self {
            SpawnResult::Spawned(id) => Some(id),
            _ => None,
        }
    }

    /// Returns the spawned conversation id, or `None` if the result is not
    /// `SpawnedConversation`.
    fn into_conversation_id(self) -> Option<ConversationId> {
        match self {
            SpawnResult::SpawnedConversation(id) => Some(id),
            _ => None,
        }
    }

    /// Returns `true` if anything was spawned (either a session or a
    /// conversation). Existing headless-path tests use this as a coarse
    /// "did the spawn succeed?" check; interactive-path tests pair it with
    /// `into_conversation_id`.
    fn is_spawned(&self) -> bool {
        matches!(
            self,
            SpawnResult::Spawned(_) | SpawnResult::SpawnedConversation(_)
        )
    }

    /// Returns `true` only if a `Conversation` was spawned (interactive branch).
    fn is_spawned_conversation(&self) -> bool {
        matches!(self, SpawnResult::SpawnedConversation(_))
    }
}

#[derive(Clone, Debug)]
pub struct SpawnAttempt {
    status: StatusKey,
    attempts: i32,
    children_snapshot: HashMap<IssueId, VersionNumber>,
}

pub struct AgentQueue {
    pub agent: crate::domain::agents::Agent,
    spawn_attempts: SharedSpawnAttempts,
}

impl AgentQueue {
    pub fn new(agent: crate::domain::agents::Agent, spawn_attempts: SharedSpawnAttempts) -> Self {
        Self {
            agent,
            spawn_attempts,
        }
    }

    /// Headless branch: build a `Session` for `issue` and persist it via
    /// `AppState::create_session`. Used when the issue's resolved status
    /// definition has `interactive: false`.
    async fn build_session_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> anyhow::Result<Option<SessionId>> {
        let session_settings = resolve_session_settings_for_issue(state, issue).await;

        // Pre-resolve the mount_spec via `mount_spec_from_create_request`. The
        // server-side `create_session` defaulting from `session_settings` is
        // bypassed because we send a non-empty `mount_spec` on the request.
        let mount_spec = state
            .resolve_mount_spec(&session_settings)
            .await
            .context("failed to resolve mount_spec for issue task")?;

        let image = session_settings
            .image
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let mut env_vars = HashMap::new();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string());

        // The `agents` domain object holds the name as a free `String`.
        // Validate here so a malformed stored name surfaces immediately
        // rather than letting the server emit a 4xx mid-spawn.
        let agent_name = hydra_common::api::v1::agents::AgentName::try_new(self.agent.name.clone())
            .with_context(|| {
                format!("agent '{}' has invalid name in the store", self.agent.name)
            })?;
        // `AgentSpec::Named` makes the server resolve the prompt /
        // mcp_config / secrets and stamp `AGENT_NAME_ENV_VAR` — no more
        // per-caller duplication of that logic.
        let request = api::sessions::CreateSessionRequest {
            mode: api::sessions::SessionMode::Headless,
            agent_config: api::sessions::AgentSpec::Named { name: agent_name },
            model: session_settings.model.clone(),
            mount_spec,
            image,
            env_vars,
            cpu_limit: session_settings.cpu_limit.clone(),
            memory_limit: session_settings.memory_limit.clone(),
            secrets: session_settings.secrets.clone(),
            spawned_from: Some(issue_id.clone()),
            resumed_from: None,
        };

        let system_actor = ActorRef::System {
            worker_name: "agent_queue".into(),
            on_behalf_of: None,
        };
        let (session_id, _session) = state
            .create_session(request, system_actor, issue.creator.clone())
            .await
            .context("failed to create session via AppState::create_session")?;

        Ok(Some(session_id))
    }

    /// Interactive branch: create a `Conversation` linked back to the issue.
    /// The companion session is materialized asynchronously by
    /// `SpawnConversationSessionsAutomation`, which inherits `spawned_from`
    /// and stamps `HYDRA_ISSUE_ID` on the session env.
    async fn build_conversation_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> anyhow::Result<Option<ConversationId>> {
        let session_settings = resolve_session_settings_for_issue(state, issue).await;

        let agent_name = hydra_common::api::v1::agents::AgentName::try_new(self.agent.name.clone())
            .with_context(|| {
                format!("agent '{}' has invalid name in the store", self.agent.name)
            })?;

        let system_actor = ActorRef::System {
            worker_name: "agent_queue".into(),
            on_behalf_of: None,
        };
        let (conversation_id, _versioned) = state
            .create_conversation(
                None,
                Some(agent_name),
                session_settings,
                Some(issue_id.clone()),
                Some(issue.title.clone()),
                system_actor,
                issue.creator.clone(),
            )
            .await
            .context("failed to create conversation via AppState::create_conversation")?;

        Ok(Some(conversation_id))
    }

    async fn register_spawn_attempt(
        &self,
        issue_id: &IssueId,
        status: StatusKey,
        children_snapshot: HashMap<IssueId, VersionNumber>,
        max_tries: i32,
    ) -> bool {
        let mut attempts = self.spawn_attempts.write().await;
        let entry = attempts.entry(issue_id.clone()).or_insert(SpawnAttempt {
            status: status.clone(),
            attempts: 0,
            children_snapshot: HashMap::new(),
        });

        let status_changed = entry.status != status;
        let children_changed = entry.children_snapshot != children_snapshot;

        if status_changed || children_changed {
            *entry = SpawnAttempt {
                status,
                attempts: 0,
                children_snapshot,
            };
        }

        if entry.attempts >= max_tries {
            return false;
        }

        entry.attempts += 1;
        true
    }

    fn max_tries_for_issue(&self, issue: &Issue) -> i32 {
        issue
            .session_settings
            .max_retries
            .map(|v| v as i32)
            .unwrap_or(self.agent.max_tries)
    }

    /// Check whether a single issue is eligible for spawning and, if so,
    /// build and return the session. This is the per-issue counterpart of the
    /// bulk polling method and is intended for event-driven automations that
    /// already know which issue to evaluate.
    pub(crate) async fn spawn_for_issue(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
        task_state: &AgentTaskState,
    ) -> anyhow::Result<SpawnResult> {
        // Assignment check: compare against the typed
        // `Principal::Agent { name }` — bare-string matching is gone.
        // Issues assigned to a `Principal::User { name == agent.name }`
        // (the typo direction) are deliberately NOT picked up.
        // Unassigned issues are NOT picked up here. Issues that should be
        // routed somewhere must be assigned explicitly (or via a per-status
        // `on_enter.assign_to` automation; see `apply_status_on_enter`).
        let is_assignment_match = matches!(
            issue.assignee.as_ref(),
            Some(hydra_common::principal::Principal::Agent { name })
                if name.as_str() == self.agent.name
        );
        if !is_assignment_match {
            return Ok(SpawnResult::Skipped);
        }

        let is_ready = state
            .is_issue_ready(issue_id)
            .await
            .context("failed to determine if issue is ready")?;
        let has_active_session = task_state.existing_issue_ids.contains(issue_id);
        let has_active_conv = has_active_conversation(state, issue_id).await?;
        let parent_running = parent_has_running_task(state, issue).await?;
        // Resolve the issue's status once: used both for the per-status
        // cap check below and for the interactive dispatch further down.
        // An unresolvable status falls back to a non-interactive branch
        // with the cap left off (preserving prior behavior for malformed
        // rows).
        let resolved_status = match state.resolve_status(issue).await {
            Ok(def) => Some(def),
            Err(err) => {
                tracing::warn!(
                    agent = self.agent.name,
                    issue_id = %issue_id,
                    status = %issue.status,
                    error = %err,
                    "failed to resolve issue status; defaulting to non-interactive headless branch"
                );
                None
            }
        };
        let interactive = resolved_status
            .as_ref()
            .map(|def| def.interactive)
            .unwrap_or(false);

        // Pick the matching cap based on the resolved status's
        // interactive flag — interactive (conversation-mode) and headless
        // sessions are capped independently per agent so a SWE agent can
        // accept N live previews without throttling its headless work
        // (or vice versa).
        let (max_simultaneous, active_tasks) = if interactive {
            (
                self.agent.max_simultaneous_interactive as usize,
                task_state.running_interactive + task_state.pending_interactive,
            )
        } else {
            (
                self.agent.max_simultaneous_headless as usize,
                task_state.running_headless + task_state.pending_headless,
            )
        };
        let at_capacity = max_simultaneous == 0 || active_tasks >= max_simultaneous;
        let status_at_capacity = if let Some(def) = resolved_status.as_ref() {
            match def.max_simultaneous_sessions {
                Some(cap) => {
                    let in_status = state
                        .store
                        .count_active_sessions_in_status(&issue.project_id, &def.key)
                        .await
                        .context("failed to count active sessions for status cap")?;
                    in_status >= cap as u64
                }
                None => false,
            }
        } else {
            false
        };

        // The conversation gate is the sibling of `has_active_session`: it
        // blocks any spawn (headless or interactive) when a live
        // conversation already exists for this issue.
        if at_capacity
            || status_at_capacity
            || has_active_session
            || has_active_conv
            || !is_ready
            || parent_running
        {
            return Ok(SpawnResult::Skipped);
        }

        // Spawn attempt tracking is checked BEFORE creating the session so
        // an exhausted issue doesn't accumulate orphan rows. (Previously the
        // session was built first; with `build_task` now persisting through
        // `create_session`, we have to flip the order or risk a row per
        // retry-exhausted attempt.)
        let max_tries = self.max_tries_for_issue(issue);
        let children_snapshot = {
            let child_ids = state.get_issue_children(issue_id).await.unwrap_or_default();
            let mut snapshot = HashMap::new();
            for child_id in child_ids {
                if let Ok(child) = state.get_issue(&child_id, false).await {
                    snapshot.insert(child_id, child.version);
                }
            }
            snapshot
        };
        if !self
            .register_spawn_attempt(issue_id, issue.status.clone(), children_snapshot, max_tries)
            .await
        {
            return Ok(SpawnResult::RetriesExhausted {
                issue_id: issue_id.clone(),
            });
        }

        // Dispatch on the resolved status's `interactive` flag. A non-
        // interactive status (or an unresolvable status) runs the legacy
        // headless branch; an interactive status mints a `Conversation`
        // whose companion session is materialized asynchronously by
        // `SpawnConversationSessionsAutomation`.
        if interactive {
            let maybe_conversation_id =
                self.build_conversation_task(state, issue_id, issue).await?;
            let Some(conversation_id) = maybe_conversation_id else {
                return Ok(SpawnResult::Skipped);
            };
            return Ok(SpawnResult::SpawnedConversation(conversation_id));
        }

        let maybe_session_id = self.build_session_task(state, issue_id, issue).await?;
        let Some(session_id) = maybe_session_id else {
            return Ok(SpawnResult::Skipped);
        };

        Ok(SpawnResult::Spawned(session_id))
    }
}

/// Returns `true` if a non-Closed (`Active` or `Idle`) conversation already
/// exists for `issue_id`. Mirrors `has_active_session` but on the
/// conversation domain; applies to both spawn branches so a live interactive
/// conversation blocks a parallel headless spawn (and vice versa).
pub(crate) async fn has_active_conversation(
    state: &AppState,
    issue_id: &IssueId,
) -> Result<bool, StoreError> {
    let query = SearchConversationsQuery {
        spawned_from: Some(issue_id.clone()),
        include_archived: Some(false),
        ..Default::default()
    };
    let conversations = state.store.list_conversations(&query).await?;
    Ok(conversations
        .iter()
        .any(|(_, versioned)| versioned.item.status != ConversationStatus::Closed))
}

pub(crate) struct AgentTaskState {
    pub(crate) existing_issue_ids: HashSet<IssueId>,
    pub(crate) running_interactive: usize,
    pub(crate) running_headless: usize,
    pub(crate) pending_interactive: usize,
    pub(crate) pending_headless: usize,
}

pub(crate) async fn agent_task_state(
    state: &AppState,
    agent_name: &str,
) -> Result<AgentTaskState, StoreError> {
    let mut task_state = AgentTaskState {
        existing_issue_ids: HashSet::new(),
        running_interactive: 0,
        running_headless: 0,
        pending_interactive: 0,
        pending_headless: 0,
    };
    let mut query = SearchSessionsQuery::default();
    query.status = vec![
        Status::Created.into(),
        Status::Pending.into(),
        Status::Running.into(),
    ];
    let sessions = state.list_sessions_with_query(&query).await?;

    for (_session_id, versioned_session) in sessions {
        let session = &versioned_session.item;
        if !matches!(
            session.env_vars.get(AGENT_NAME_ENV_VAR),
            Some(current) if current == agent_name
        ) {
            continue;
        }

        // Bucket by `SessionMode`: interactive (conversation-mode) and
        // headless sessions are capped independently. The mode is
        // first-class on the persisted session, so there's no ambiguity
        // here.
        let is_interactive = session.is_interactive();
        match (session.status, is_interactive) {
            (Status::Created, true) => task_state.pending_interactive += 1,
            (Status::Created, false) => task_state.pending_headless += 1,
            (Status::Pending | Status::Running, true) => task_state.running_interactive += 1,
            (Status::Pending | Status::Running, false) => task_state.running_headless += 1,
            _ => {}
        }

        if let Some(issue_id) = session
            .env_vars
            .get(ISSUE_ID_ENV_VAR)
            .and_then(|value| value.parse::<IssueId>().ok())
        {
            task_state.existing_issue_ids.insert(issue_id);
        }
    }

    Ok(task_state)
}

/// Resolve the effective [`crate::domain::issues::SessionSettings`] for
/// `issue` by merging issue-level overrides over the per-status overrides
/// on the issue's resolved [`StatusDefinition`] over the per-project
/// overrides on the issue's project, then applying the global defaults
/// via `apply_session_settings_defaults`.
///
/// Precedence (most → least specific): `issue.session_settings` →
/// `status.session_settings` → `project.session_settings` → global
/// default. `SessionSettings::merge` is None-fill (primary wins
/// per-field), so each field falls through the layers until one supplies
/// a `Some`. A `resolve_status` or `get_project` failure degrades by
/// skipping just that layer — the spawn dispatcher already emits a
/// warning for the status branch, and we don't want a stale project
/// lookup to break headless spawning.
pub(crate) async fn resolve_session_settings_for_issue(
    state: &AppState,
    issue: &Issue,
) -> crate::domain::issues::SessionSettings {
    let status_settings: crate::domain::issues::SessionSettings = match state
        .resolve_status(issue)
        .await
    {
        Ok(def) => def.session_settings.into(),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "failed to resolve status for issue {}; status-level session_settings unavailable",
                issue.title
            );
            crate::domain::issues::SessionSettings::default()
        }
    };
    let project_settings: crate::domain::issues::SessionSettings = match state
        .store
        .get_project(&issue.project_id, false)
        .await
    {
        Ok(versioned) => versioned.item.session_settings.into(),
        Err(err) => {
            tracing::warn!(
                error = %err,
                project_id = %issue.project_id,
                "failed to load project for issue {}; project-level session_settings unavailable",
                issue.title
            );
            crate::domain::issues::SessionSettings::default()
        }
    };
    let merged = crate::domain::issues::SessionSettings::merge(
        issue.session_settings.clone(),
        status_settings,
    );
    let merged = crate::domain::issues::SessionSettings::merge(merged, project_settings);
    state.apply_session_settings_defaults(merged)
}

/// Returns `true` if any `ChildOf` parent of `issue` has a *headless*
/// session in `Pending`/`Running` state.
///
/// Interactive parent sessions are explicitly exempted: an
/// `Interactive`-mode session represents user-facing chat work (e.g. a PM
/// agent chatting with a user on a `specification`-status issue) and
/// should not gate child issues from spawning. Headless sessions still
/// gate, since they represent autonomous work that must serialize
/// against children.
pub(crate) async fn parent_has_running_task(
    state: &AppState,
    issue: &Issue,
) -> Result<bool, StoreError> {
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
    {
        for task_id in state.get_sessions_for_issue(&dependency.issue_id).await? {
            let session = state.get_session(&task_id).await?;
            if session.is_interactive() {
                continue;
            }
            if matches!(session.status, Status::Pending | Status::Running) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::SessionSettings;
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::domain::sessions::SessionMode;
    use crate::{
        app::Repository,
        config::{DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
        routes::sessions::mount_spec_from_create_request,
        test::{TestStateHandles, test_state_handles, test_state_with_repo_handles},
    };
    use chrono::Utc;
    use hydra_common::api::v1::sessions::Bundle;

    fn default_user() -> Username {
        Username::from("spawner")
    }

    fn shared_attempts() -> SharedSpawnAttempts {
        Arc::new(RwLock::new(HashMap::new()))
    }

    fn make_agent(agent_name: &str) -> crate::domain::agents::Agent {
        crate::domain::agents::Agent::new(
            agent_name.to_string(),
            format!("/agents/{agent_name}/prompt.md"),
            None,
            DEFAULT_AGENT_MAX_TRIES,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            false,
            Vec::new(),
        )
    }

    fn queue(agent_name: &str) -> AgentQueue {
        queue_with_attempts(agent_name, shared_attempts())
    }

    fn queue_with_attempts(agent_name: &str, attempts: SharedSpawnAttempts) -> AgentQueue {
        AgentQueue::new(make_agent(agent_name), attempts)
    }

    fn queue_with_secrets(agent_name: &str, secrets: Vec<String>) -> AgentQueue {
        use crate::domain::agents::Agent;
        AgentQueue::new(
            Agent::new(
                agent_name.to_string(),
                format!("/agents/{agent_name}/prompt.md"),
                None,
                DEFAULT_AGENT_MAX_TRIES,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                false,
                secrets,
            ),
            shared_attempts(),
        )
    }

    /// Seed the agent's prompt document AND the agent record so the
    /// server's `AgentSpec::Named` branch can resolve both. The agent
    /// record is inserted using the same shape as the in-memory
    /// `AgentQueue.agent` so `queue(agent_name)`-built tests see a
    /// consistent agent across the server's read path and the
    /// queue's in-memory copy.
    async fn seed_agent_prompt(
        handles: &TestStateHandles,
        agent_name: &str,
        prompt: &str,
    ) -> anyhow::Result<()> {
        seed_agent(handles, make_agent(agent_name), prompt).await
    }

    async fn seed_agent_with_secrets(
        handles: &TestStateHandles,
        agent_name: &str,
        prompt: &str,
        secrets: Vec<String>,
    ) -> anyhow::Result<()> {
        use crate::domain::agents::Agent;
        seed_agent(
            handles,
            Agent::new(
                agent_name.to_string(),
                format!("/agents/{agent_name}/prompt.md"),
                None,
                DEFAULT_AGENT_MAX_TRIES,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                false,
                secrets,
            ),
            prompt,
        )
        .await
    }

    async fn seed_agent(
        handles: &TestStateHandles,
        agent: crate::domain::agents::Agent,
        prompt: &str,
    ) -> anyhow::Result<()> {
        use crate::domain::documents::Document;
        let path = agent.prompt_path.clone();
        let doc = Document {
            title: format!("{} prompt", agent.name),
            body_markdown: prompt.to_string(),
            path: Some(path.parse().unwrap()),
            archived: false,
        };
        // The document may already exist if a previous call seeded
        // this agent (state_with_repository seeds vanilla copies of
        // `agent-a` and `agent-b`; bespoke tests re-seed afterwards).
        // Tolerate the conflict — the test only cares about the agent
        // record's fields, not the prompt body, and the prompt is the
        // same string in practice.
        let _ = handles.store.add_document(doc, &ActorRef::test()).await;
        // Same idempotency story for the agent record.
        if handles.store.add_agent(agent.clone()).await.is_err() {
            handles.store.update_agent(agent).await?;
        }
        Ok(())
    }

    fn repository() -> (RepoName, Repository) {
        let repo_name = RepoName::from_str("dourolabs/hydra").expect("repo name should parse");
        let repository = Repository::new(
            "https://github.com/dourolabs/hydra.git".to_string(),
            Some("main".to_string()),
        );

        (repo_name, repository)
    }

    fn session_settings(repo_name: &RepoName) -> SessionSettings {
        SessionSettings {
            repo_name: Some(repo_name.clone()),
            image: Some("repo-image".to_string()),
            ..SessionSettings::default()
        }
    }

    async fn state_with_repository() -> anyhow::Result<(TestStateHandles, RepoName)> {
        let (repo_name, repository) = repository();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository).await?;
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;
        seed_agent_prompt(&handles, "agent-b", "Fix the issue").await?;
        Ok((handles, repo_name))
    }

    async fn record_completed_task(
        handles: &TestStateHandles,
        task_id: SessionId,
    ) -> anyhow::Result<()> {
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await?;
        handles
            .state
            .transition_task_to_running(&task_id, ActorRef::test())
            .await?;
        handles
            .state
            .transition_task_to_completion(&task_id, Ok(()), None, None, ActorRef::test())
            .await?;
        Ok(())
    }

    /// Construct a `Principal::Agent` from a `&str` for test fixtures.
    /// The assignment check is typed-equality, so test issues need a
    /// `Principal::Agent { name: "..." }` value (not a bare string) to
    /// be picked up by the matching agent's queue.
    fn test_agent_principal(name: &str) -> hydra_common::principal::Principal {
        hydra_common::principal::Principal::Agent {
            name: hydra_common::api::v1::agents::AgentName::try_new(name)
                .expect("test agent name should validate"),
        }
    }

    fn issue_with_type(
        issue_type: IssueType,
        description: &str,
        status: StatusKey,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        repo_name: &RepoName,
    ) -> Issue {
        Issue::new(
            issue_type,
            "Test Title".to_string(),
            description.to_string(),
            default_user(),
            status,
            crate::domain::projects::default_project_id(),
            assignee.map(test_agent_principal),
            Some(session_settings(repo_name)),
            dependencies,
            Vec::new(),
            None,
            None,
        )
    }

    fn issue(
        description: &str,
        status: StatusKey,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        repo_name: &RepoName,
    ) -> Issue {
        issue_with_type(
            IssueType::Task,
            description,
            status,
            assignee,
            dependencies,
            repo_name,
        )
    }

    fn issue_without_repo(description: &str, status: StatusKey, assignee: Option<&str>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: description.to_string(),
            creator: default_user(),
            status,
            project_id: crate::domain::projects::default_project_id(),
            assignee: assignee.map(test_agent_principal),
            session_settings: SessionSettings::default(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            archived: false,
            form: None,
            form_response: None,
        }
    }

    fn task(
        prompt: &str,
        bundle: Bundle,
        spawned_from: Option<IssueId>,
        image: Option<&str>,
        env_vars: HashMap<String, String>,
    ) -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session::new(
            Username::from("test-creator"),
            spawned_from,
            None,
            AgentConfig::new(None, None, Some(prompt.to_string()), None),
            mount_spec_from_create_request(bundle, None),
            image.map(str::to_string),
            env_vars,
            None,
            None,
            None,
            SessionMode::Headless,
            Status::Created,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn spawns_tasks_for_ready_assigned_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (assigned_issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Fix login page",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let (in_progress_issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "In-progress but ready",
                    status("in-progress"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Closed issues carry no assignee under the default-project
        // invariant (`apply_status_on_enter.clear_assignee` runs on
        // every transition into `closed`), and a None-assignee issue
        // is in no agent queue.
        handles
            .store
            .add_issue(
                issue("Ignore closed", status("closed"), None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issues = handles.state.list_issues().await?;

        let mut session_ids = Vec::new();
        for (issue_id, versioned_issue) in &issues {
            if let Ok(SpawnResult::Spawned(id)) = queue
                .spawn_for_issue(&handles.state, issue_id, &versioned_issue.item, &task_state)
                .await
            {
                session_ids.push(id);
            }
        }
        assert_eq!(session_ids.len(), 2);

        let mut issue_ids = HashSet::new();
        let mut spawned_from_issue_ids = HashSet::new();
        let _ = repo_name.clone();
        for id in session_ids {
            let task = handles.state.get_session(&id).await?;
            let Session {
                mode,
                spawned_from,
                env_vars,
                agent_config,
                ..
            } = task;

            assert!(matches!(&mode, SessionMode::Headless));
            assert_eq!(agent_config.system_prompt.as_deref(), Some("Fix the issue"));
            spawned_from_issue_ids.insert(spawned_from);
            issue_ids.insert(env_vars.get(ISSUE_ID_ENV_VAR).cloned());
            assert_eq!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(&"agent-a".to_string())
            );
        }

        let expected = HashSet::from([
            Some(assigned_issue_id.to_string()),
            Some(in_progress_issue_id.to_string()),
        ]);
        assert_eq!(issue_ids, expected);
        assert_eq!(
            spawned_from_issue_ids,
            HashSet::from([Some(assigned_issue_id), Some(in_progress_issue_id),])
        );

        Ok(())
    }

    #[tokio::test]
    async fn does_not_requeue_when_task_exists() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Already queued",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        handles
            .store
            .add_session(
                task(
                    "Fix the issue",
                    Bundle::None,
                    Some(issue_id.clone()),
                    Some("hydra-worker:latest"),
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn task_requeues_after_changes_requested_patch_update() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let patch = Patch::new(
            "Review patch".to_string(),
            "Review patch description".to_string(),
            "diff --git a/file b/file\n".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = handles.store.add_patch(patch, &ActorRef::test()).await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Review patch".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: vec![patch_id.clone()],
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        let updated_patch = handles.store.get_patch(&patch_id, false).await?;
        let mut updated_patch = updated_patch.item;
        updated_patch.status = PatchStatus::ChangesRequested;
        updated_patch.diff = "diff --git a/file b/file\n+change\n".to_string();
        updated_patch.reviews = vec![Review::new(
            "needs adjustments".to_string(),
            false,
            hydra_common::Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap(),
            },
            None,
        )];
        handles
            .store
            .update_patch(&patch_id, updated_patch, &ActorRef::test())
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn task_assignee_mismatch_skips_for_non_assignee() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue_with_type(
                    IssueType::Task,
                    "Task",
                    status("open"),
                    Some("pm"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-b");
        let task_state = agent_task_state(&handles.state, "agent-b").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn non_assignment_agent_skips_unassigned_issue() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue_without_repo("Needs assignment", status("open"), None),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_issue_not_ready() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (blocker_id, _) = handles
            .store
            .add_issue(
                issue("Blocker", status("open"), None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Blocked issue",
                    status("open"),
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id.clone(),
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_parent_task_running() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Parent issue",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let (task_id, _) = handles
            .store
            .add_session(
                task(
                    "Parent task",
                    Bundle::None,
                    Some(parent_id.clone()),
                    Some("hydra-worker:latest"),
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), parent_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await?;

        let (child_issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Child issue",
                    status("open"),
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&child_issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &child_issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    /// Add a session for `parent_id`, optionally rewriting its mode to
    /// `SessionMode::Interactive`, and transition it to the requested
    /// status (`Pending` or `Running`). Returns the session id so a
    /// caller can mix multiple parent sessions in one test.
    async fn seed_parent_session(
        handles: &TestStateHandles,
        parent_id: &IssueId,
        interactive: bool,
        target_status: Status,
    ) -> anyhow::Result<SessionId> {
        use crate::domain::sessions::SessionMode;

        let mut session = task(
            "Parent task",
            Bundle::None,
            Some(parent_id.clone()),
            Some("hydra-worker:latest"),
            HashMap::from([
                (ISSUE_ID_ENV_VAR.to_string(), parent_id.to_string()),
                (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
            ]),
        );
        if interactive {
            session.mode = SessionMode::Interactive {
                conversation_id: ConversationId::new(),
                idle_timeout: None,
                greet_user: false,
            };
        }

        let (task_id, _) = handles
            .store
            .add_session(session, Utc::now(), &ActorRef::test())
            .await?;
        match target_status {
            Status::Pending => {
                handles
                    .state
                    .transition_task_to_pending(&task_id, ActorRef::test())
                    .await?;
            }
            Status::Running => {
                handles
                    .state
                    .transition_task_to_running(&task_id, ActorRef::test())
                    .await?;
            }
            other => panic!("seed_parent_session only supports Pending/Running, got {other:?}"),
        }
        Ok(task_id)
    }

    async fn add_parent_and_child(
        handles: &TestStateHandles,
        repo_name: &RepoName,
        parent_title: &str,
        child_title: &str,
    ) -> anyhow::Result<(IssueId, IssueId)> {
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    parent_title,
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let (child_id, _) = handles
            .store
            .add_issue(
                issue(
                    child_title,
                    status("open"),
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        Ok((parent_id, child_id))
    }

    #[tokio::test]
    async fn parent_has_running_task_blocks_when_parent_headless_pending() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, child_id) =
            add_parent_and_child(&handles, &repo_name, "Parent", "Child").await?;
        seed_parent_session(&handles, &parent_id, false, Status::Pending).await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn parent_has_running_task_blocks_when_parent_headless_running() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, child_id) =
            add_parent_and_child(&handles, &repo_name, "Parent", "Child").await?;
        seed_parent_session(&handles, &parent_id, false, Status::Running).await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn parent_has_running_task_ignores_interactive_pending() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, child_id) =
            add_parent_and_child(&handles, &repo_name, "Parent", "Child").await?;
        seed_parent_session(&handles, &parent_id, true, Status::Pending).await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(!parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn parent_has_running_task_ignores_interactive_running() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, child_id) =
            add_parent_and_child(&handles, &repo_name, "Parent", "Child").await?;
        seed_parent_session(&handles, &parent_id, true, Status::Running).await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(!parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn parent_has_running_task_blocks_when_headless_and_interactive_running()
    -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, child_id) =
            add_parent_and_child(&handles, &repo_name, "Parent", "Child").await?;
        seed_parent_session(&handles, &parent_id, false, Status::Running).await?;
        seed_parent_session(&handles, &parent_id, true, Status::Running).await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn parent_has_running_task_blocks_when_any_parent_has_headless_running()
    -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        let (interactive_parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Interactive parent",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let (headless_parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Headless parent",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        seed_parent_session(&handles, &interactive_parent_id, true, Status::Running).await?;
        seed_parent_session(&handles, &headless_parent_id, false, Status::Running).await?;

        let (child_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Child of both",
                    status("open"),
                    Some("agent-a"),
                    vec![
                        IssueDependency::new(
                            IssueDependencyType::ChildOf,
                            interactive_parent_id.clone(),
                        ),
                        IssueDependency::new(
                            IssueDependencyType::ChildOf,
                            headless_parent_id.clone(),
                        ),
                    ],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let child = handles.store.get_issue(&child_id, false).await?.item;
        assert!(parent_has_running_task(&handles.state, &child).await?);

        Ok(())
    }

    #[tokio::test]
    async fn spawns_when_repo_missing_and_allows_missing_image() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id1, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Missing repo".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: None,
                        remote_url: None,
                        image: Some("hydra-worker:latest".to_string()),
                        model: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: None,
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let (issue_id2, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Missing image".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        remote_url: None,
                        image: None,
                        model: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: None,
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue1 = handles.store.get_issue(&issue_id1, false).await?.item;
        let result1 = queue
            .spawn_for_issue(&handles.state, &issue_id1, &issue1, &task_state)
            .await?;
        assert!(result1.is_spawned());

        let issue2 = handles.store.get_issue(&issue_id2, false).await?.item;
        let result2 = queue
            .spawn_for_issue(&handles.state, &issue_id2, &issue2, &task_state)
            .await?;
        assert!(result2.is_spawned());

        let session2 = handles
            .state
            .get_session(&result2.into_session_id().unwrap())
            .await?;
        // PR-F: `Session.context` is gone; the resolved bundle lives on
        // `session.mount_spec.mounts[0]` (no build_cache in this test → just
        // a Bundle::GitRepository with the service-repo URL).
        use hydra_common::api::v1::sessions::MountItem;
        let expected_url = "https://github.com/dourolabs/hydra.git";
        let first_bundle = session2.mount_spec.mounts.iter().find_map(|m| match m {
            MountItem::Bundle { bundle, .. } => Some(bundle.clone()),
            _ => None,
        });
        let has_repo_task = matches!(
            first_bundle.as_ref(),
            Some(Bundle::GitRepository { url, .. }) if url == expected_url
        );
        let _ = repo_name;
        assert!(
            has_repo_task,
            "expected a task with the resolved service-repo URL, got {first_bundle:?}",
        );

        let session1 = handles
            .state
            .get_session(&result1.into_session_id().unwrap())
            .await?;
        // Repo-less issue → the spec carries a `Bundle::None` Bundle mount
        // (alongside the standard Documents mount) so the worker creates
        // `working_dir` on disk and `current_dir()` doesn't ENOENT
        // (see i-lkadrfky). Pre-fix this was `mounts: []`.
        assert!(
            matches!(
                session1.mount_spec.mounts.first(),
                Some(MountItem::Bundle {
                    bundle: Bundle::None,
                    ..
                })
            ),
            "expected a Bundle::None mount for a repo-less issue; got {:?}",
            session1.mount_spec.mounts,
        );

        Ok(())
    }

    #[tokio::test]
    async fn uses_session_settings_max_retries_override() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 3;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Override retries".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name),
                        remote_url: None,
                        image: Some("hydra-worker:latest".to_string()),
                        model: None,
                        branch: Some("main".to_string()),
                        max_retries: Some(1),
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: None,
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_at_max_simultaneous_headless() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous_headless = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Already running".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let (task_id, _) = handles
            .store
            .add_session(
                task(
                    "Existing",
                    Bundle::None,
                    Some(issue_id.clone()),
                    None,
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn caps_new_tasks_to_remaining_capacity() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous_headless = 2;

        let (handles, repo_name) = state_with_repository().await?;
        let (first_issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "First issue".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;
        let (second_issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Second issue".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        handles
            .store
            .add_session(
                task(
                    "Pending work",
                    Bundle::None,
                    Some(first_issue_id.clone()),
                    None,
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), first_issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&second_issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &second_issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        let spawned_session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert_eq!(
            spawned_session
                .env_vars
                .get(ISSUE_ID_ENV_VAR)
                .map(String::as_str),
            Some(second_issue_id.as_ref())
        );

        Ok(())
    }

    /// With a tight interactive cap, a saturating interactive session
    /// blocks a second interactive spawn while leaving the headless cap
    /// untouched: a headless spawn still proceeds.
    #[tokio::test]
    async fn interactive_cap_blocks_interactive_but_not_headless() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, backlog_key) = seed_interactive_project(&handles).await;

        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous_interactive = 1;
        queue.agent.max_simultaneous_headless = 5;

        // Saturate the interactive cap with an active interactive
        // session for an unrelated issue.
        let (saturating_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(
                    &project_id,
                    &interactive_key,
                    &repo_name,
                    "saturating interactive",
                ),
                &ActorRef::test(),
            )
            .await?;
        let _ = seed_parent_session(&handles, &saturating_issue_id, true, Status::Running).await?;

        // A second interactive issue should be deferred.
        let (interactive_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(
                    &project_id,
                    &interactive_key,
                    &repo_name,
                    "blocked interactive",
                ),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles
            .store
            .get_issue(&interactive_issue_id, false)
            .await?
            .item;
        let result = queue
            .spawn_for_issue(
                &handles.state,
                &interactive_issue_id,
                &issue_item,
                &task_state,
            )
            .await?;
        assert!(
            matches!(result, SpawnResult::Skipped),
            "interactive cap saturated must defer the second interactive spawn"
        );

        // A headless issue on the same agent should still spawn — the
        // headless cap (5) is untouched by the interactive saturation.
        let (headless_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &backlog_key, &repo_name, "still-ok headless"),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles
            .store
            .get_issue(&headless_issue_id, false)
            .await?
            .item;
        let result = queue
            .spawn_for_issue(&handles.state, &headless_issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            result.is_spawned(),
            "headless cap untouched by interactive saturation — spawn must proceed"
        );

        Ok(())
    }

    /// With a tight headless cap, a saturating headless session blocks
    /// a second headless spawn while leaving the interactive cap
    /// untouched: an interactive spawn still proceeds.
    #[tokio::test]
    async fn headless_cap_blocks_headless_but_not_interactive() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, backlog_key) = seed_interactive_project(&handles).await;

        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous_interactive = 5;
        queue.agent.max_simultaneous_headless = 1;

        // Saturate the headless cap with an active headless session for
        // an unrelated issue.
        let (saturating_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &backlog_key, &repo_name, "saturating headless"),
                &ActorRef::test(),
            )
            .await?;
        let _ = seed_parent_session(&handles, &saturating_issue_id, false, Status::Running).await?;

        // A second headless issue should be deferred.
        let (headless_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &backlog_key, &repo_name, "blocked headless"),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles
            .store
            .get_issue(&headless_issue_id, false)
            .await?
            .item;
        let result = queue
            .spawn_for_issue(&handles.state, &headless_issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            matches!(result, SpawnResult::Skipped),
            "headless cap saturated must defer the second headless spawn"
        );

        // An interactive issue on the same agent should still spawn —
        // the interactive cap (5) is untouched by the headless
        // saturation.
        let (interactive_issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(
                    &project_id,
                    &interactive_key,
                    &repo_name,
                    "still-ok interactive",
                ),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles
            .store
            .get_issue(&interactive_issue_id, false)
            .await?
            .item;
        let result = queue
            .spawn_for_issue(
                &handles.state,
                &interactive_issue_id,
                &issue_item,
                &task_state,
            )
            .await?;
        assert!(
            result.is_spawned_conversation(),
            "interactive cap untouched by headless saturation — spawn must proceed"
        );

        Ok(())
    }

    #[tokio::test]
    async fn enforces_max_spawn_attempts_per_state() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 2;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Retry limited",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_status_changes() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "State change reset",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let first_run = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(first_run.is_spawned());
        // Transition the spawned session to terminal so the
        // `has_active_session` guard doesn't block the next iteration. PR-E
        // moved storage inside `build_task`, so sessions accumulate in the
        // store between spawn calls and need to be drained here.
        record_completed_task(&handles, first_run.into_session_id().unwrap()).await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let blocked = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!blocked.is_spawned());

        let issue_ver = handles.store.get_issue(&issue_id, false).await?;
        let mut issue_item = issue_ver.item;
        issue_item.status = status("in-progress");
        handles
            .store
            .update_issue(&issue_id, issue_item, &ActorRef::test())
            .await?;

        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_child_created() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Parent with children progress",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // First spawn attempt succeeds (attempt 1 of 1).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&parent_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &parent_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        // Second attempt should be blocked (max_tries=1 reached).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&parent_id, false).await?.item;
        let blocked = queue
            .spawn_for_issue(&handles.state, &parent_id, &issue_item, &task_state)
            .await?;
        assert!(!blocked.is_spawned());

        // Create a terminal (Closed) child issue — counts as progress on
        // the parent, and a `unblocks_parents=true` child keeps the
        // parent ready under the unified readiness rule.
        // Assign to a different agent so it doesn't spawn here.
        handles
            .store
            .add_issue(
                issue(
                    "Child issue",
                    status("closed"),
                    Some("agent-b"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Now the counter should have reset, so spawning succeeds again.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&parent_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &parent_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        let spawned_session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert_eq!(
            spawned_session
                .env_vars
                .get(ISSUE_ID_ENV_VAR)
                .map(String::as_str),
            Some(parent_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_child_updated() -> anyhow::Result<()> {
        // Under the unified readiness rule, a parent is only ready when
        // every direct child has `unblocks_parents = true`. We use a
        // Closed child throughout so the parent stays ready; the
        // children_snapshot still changes when the child's version bumps,
        // which is what resets the parent's spawn counter.
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Parent with child update",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Create a terminal (Closed) child before the first spawn.
        let (child_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Child issue",
                    status("closed"),
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // First spawn attempt succeeds for the parent. Child is Closed
        // so the queue skips it (terminal).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let parent_issue = handles.store.get_issue(&parent_id, false).await?.item;
        let parent_result = queue
            .spawn_for_issue(&handles.state, &parent_id, &parent_issue, &task_state)
            .await?;
        assert!(parent_result.is_spawned());
        record_completed_task(&handles, parent_result.into_session_id().unwrap()).await?;

        // Further attempts should be blocked.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let parent_issue = handles.store.get_issue(&parent_id, false).await?.item;
        let blocked = queue
            .spawn_for_issue(&handles.state, &parent_id, &parent_issue, &task_state)
            .await?;
        assert!(!blocked.is_spawned());

        // Update the child's title — status stays Closed but the version
        // bumps, which counts as parent progress.
        let child = handles.store.get_issue(&child_id, false).await?;
        let mut child_item = child.item;
        child_item.title = "made further notes".to_string();
        handles
            .store
            .update_issue(&child_id, child_item, &ActorRef::test())
            .await?;

        // Parent's counter should have reset (child version changed).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let parent_issue = handles.store.get_issue(&parent_id, false).await?.item;
        let parent_result = queue
            .spawn_for_issue(&handles.state, &parent_id, &parent_issue, &task_state)
            .await?;
        assert!(parent_result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_reset_counter_when_children_unchanged() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Parent no progress",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Create a terminal (Closed) child so the parent stays ready
        // under the unified readiness rule.
        let (_child_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Child issue",
                    status("closed"),
                    Some("agent-b"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // First spawn consumes the parent.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let parent_issue = handles.store.get_issue(&parent_id, false).await?.item;
        let parent_result = queue
            .spawn_for_issue(&handles.state, &parent_id, &parent_issue, &task_state)
            .await?;
        assert!(parent_result.is_spawned());
        record_completed_task(&handles, parent_result.into_session_id().unwrap()).await?;

        // No changes to children — counter should NOT reset, so no tasks spawn.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let parent_issue = handles.store.get_issue(&parent_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &parent_id, &parent_issue, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[test]
    fn builds_from_agent() {
        use crate::domain::agents::Agent;

        let agent = Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt.md".to_string(),
            None,
            5,
            7,
            10,
            false,
            Vec::new(),
        );

        let queue = AgentQueue::new(agent, shared_attempts());

        assert_eq!(queue.agent.name, "test-agent");
        assert_eq!(queue.agent.prompt_path, "/agents/test-agent/prompt.md");
        assert_eq!(queue.agent.max_tries, 5);
        assert_eq!(queue.agent.max_simultaneous_interactive, 7);
        assert_eq!(queue.agent.max_simultaneous_headless, 10);
    }

    // Dropped/failed default-project issues carry no assignee under
    // the `apply_status_on_enter.clear_assignee` invariant — a
    // None-assignee issue is in no agent queue.
    #[tokio::test]
    async fn does_not_spawn_for_dropped_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue("Dropped issue", status("dropped"), None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_for_failed_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue("Failed issue", status("failed"), None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn service_repo_context_uses_repo_defaults() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let default_branch = repository
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".into());
        let default_image = "agent-image".to_string();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository.clone()).await?;
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Assigned".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        remote_url: None,
                        image: Some(default_image.clone()),
                        model: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: None,
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;

        let resolved = handles.state.resolve_task(&session).await?;
        // ServiceRepository round-trips through resolved_task → resolver →
        // GitRepository. The persisted `session.mount_spec` already carries
        // a fully-lowered url because agent_queue pre-resolves the mount_spec
        // before calling `create_session`; `resolved.bundle` then mirrors
        // that.
        let _ = repo_name.clone();
        assert_eq!(
            resolved.bundle,
            Bundle::GitRepository {
                url: "https://github.com/dourolabs/hydra.git".to_string(),
                rev: default_branch,
            }
        );
        assert_eq!(resolved.image, default_image);
        assert_eq!(
            resolved
                .env_vars
                .get(ISSUE_ID_ENV_VAR)
                .map(|value| value.as_str()),
            Some(issue_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn spawner_passes_secrets_from_session_settings() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository.clone()).await?;
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;
        let secrets = vec!["db-secret".to_string(), "api-key".to_string()];
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Issue with secrets".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        remote_url: None,
                        image: Some("worker:latest".to_string()),
                        model: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: Some(secrets.clone()),
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert_eq!(session.secrets, Some(secrets));

        Ok(())
    }

    #[tokio::test]
    async fn spawner_handles_none_secrets() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository.clone()).await?;
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Issue without secrets".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        remote_url: None,
                        image: Some("worker:latest".to_string()),
                        model: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                        secrets: None,
                        idle_timeout: None,
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert!(session.secrets.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn shared_spawn_attempts_persist_across_queue_instances() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Persistent counter",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Share the same spawn attempts across two AgentQueue instances
        // (simulating two scheduler iterations).
        let attempts = shared_attempts();

        let mut queue1 = queue_with_attempts("agent-a", attempts.clone());
        queue1.agent.max_tries = 2;

        // First iteration: spawns one task (attempt 1 of 2).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue1
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        // Second iteration: new AgentQueue with same shared state.
        let mut queue2 = queue_with_attempts("agent-a", attempts.clone());
        queue2.agent.max_tries = 2;

        // Should still spawn (attempt 2 of 2).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue2
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        // Third iteration: new AgentQueue, same shared state.
        let mut queue3 = queue_with_attempts("agent-a", attempts);
        queue3.agent.max_tries = 2;

        // Should be blocked (max_tries=2 reached across iterations).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue3
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            !result.is_spawned(),
            "expected no tasks after max_tries reached across queue instances"
        );

        Ok(())
    }

    #[tokio::test]
    async fn merges_agent_secrets_with_issue_secrets() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        // Create an issue with issue-level secrets.
        let mut issue_with_secrets = issue(
            "Issue with secrets",
            status("open"),
            Some("agent-a"),
            vec![],
            &repo_name,
        );
        issue_with_secrets.session_settings.secrets =
            Some(vec!["GH_TOKEN".to_string(), "OPENAI_API_KEY".to_string()]);
        let (issue_id, _) = handles
            .store
            .add_issue(issue_with_secrets, &ActorRef::test())
            .await?;

        // Agent has its own secrets, one overlapping with the issue.
        seed_agent_with_secrets(
            &handles,
            "agent-a",
            "Fix the issue",
            vec!["OPENAI_API_KEY".to_string(), "CUSTOM_KEY".to_string()],
        )
        .await?;
        let queue = queue_with_secrets(
            "agent-a",
            vec!["OPENAI_API_KEY".to_string(), "CUSTOM_KEY".to_string()],
        );
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        // Agent secrets come first, then issue secrets, deduplicated.
        assert_eq!(
            session.secrets,
            Some(vec![
                "OPENAI_API_KEY".to_string(),
                "CUSTOM_KEY".to_string(),
                "GH_TOKEN".to_string(),
            ])
        );

        Ok(())
    }

    #[tokio::test]
    async fn agent_secrets_only_when_issue_has_none() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        // Issue without secrets.
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "No issue secrets",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        seed_agent_with_secrets(
            &handles,
            "agent-a",
            "Fix the issue",
            vec!["AGENT_SECRET".to_string()],
        )
        .await?;
        let queue = queue_with_secrets("agent-a", vec!["AGENT_SECRET".to_string()]);
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert_eq!(session.secrets, Some(vec!["AGENT_SECRET".to_string()]));

        Ok(())
    }

    #[tokio::test]
    async fn no_secrets_when_agent_and_issue_have_none() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "No secrets anywhere",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned());

        let session = handles
            .state
            .get_session(&result.into_session_id().unwrap())
            .await?;
        assert_eq!(session.secrets, None);

        Ok(())
    }

    fn queue_with_mcp_config_path(agent_name: &str, mcp_config_path: &str) -> AgentQueue {
        use crate::domain::agents::Agent;
        AgentQueue::new(
            Agent::new(
                agent_name.to_string(),
                format!("/agents/{agent_name}/prompt.md"),
                Some(mcp_config_path.to_string()),
                DEFAULT_AGENT_MAX_TRIES,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                false,
                Vec::new(),
            ),
            shared_attempts(),
        )
    }

    async fn seed_mcp_config(
        handles: &TestStateHandles,
        path: &str,
        config_json: &str,
    ) -> anyhow::Result<()> {
        use crate::domain::documents::Document;
        let doc = Document {
            title: "MCP config".to_string(),
            body_markdown: config_json.to_string(),
            path: Some(path.parse().unwrap()),
            archived: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    /// Re-seed the agent in the store with `mcp_config_path` populated
    /// so the server's `AgentSpec::Named` branch resolves the same MCP
    /// config as the queue's in-memory agent.
    async fn seed_agent_with_mcp_config_path(
        handles: &TestStateHandles,
        agent_name: &str,
        mcp_config_path: &str,
    ) -> anyhow::Result<()> {
        use crate::domain::agents::Agent;
        let agent = Agent::new(
            agent_name.to_string(),
            format!("/agents/{agent_name}/prompt.md"),
            Some(mcp_config_path.to_string()),
            DEFAULT_AGENT_MAX_TRIES,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            false,
            Vec::new(),
        );
        seed_agent(handles, agent, "Fix the issue").await
    }

    #[tokio::test]
    async fn spawn_populates_mcp_config_from_agent() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let mcp_config_path = "/agents/agent-a/mcp_config.json";
        let mcp_json = r#"{"mcpServers":{"playwright":{"command":"npx","args":["@anthropic/mcp-playwright"]}}}"#;

        seed_mcp_config(&handles, mcp_config_path, mcp_json).await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Test MCP config",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        seed_agent_with_mcp_config_path(&handles, "agent-a", mcp_config_path).await?;
        let queue = queue_with_mcp_config_path("agent-a", mcp_config_path);
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;

        let session_id = result.into_session_id().expect("should spawn a session");
        let session = handles.state.get_session(&session_id).await?;
        let mcp_config = session
            .agent_config
            .mcp_config
            .expect("session should have mcp_config populated");
        let expected: serde_json::Value = serde_json::from_str(mcp_json).unwrap();
        assert_eq!(mcp_config, expected);

        Ok(())
    }

    #[tokio::test]
    async fn spawn_leaves_mcp_config_none_when_no_path() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "No MCP config",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a"); // no mcp_config_path
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;

        let session_id = result.into_session_id().expect("should spawn a session");
        let session = handles.state.get_session(&session_id).await?;
        assert!(session.agent_config.mcp_config.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn spawn_leaves_mcp_config_none_when_doc_missing() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Missing MCP doc",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Agent has mcp_config_path but the document doesn't exist
        seed_agent_with_mcp_config_path(
            &handles,
            "agent-a",
            "/agents/agent-a/nonexistent_mcp_config.json",
        )
        .await?;
        let queue =
            queue_with_mcp_config_path("agent-a", "/agents/agent-a/nonexistent_mcp_config.json");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;

        let session_id = result.into_session_id().expect("should spawn a session");
        let session = handles.state.get_session(&session_id).await?;
        assert!(
            session.agent_config.mcp_config.is_none(),
            "mcp_config should be None when document is missing"
        );

        Ok(())
    }

    /// Regression for §2.5: agent_queue spawns must go through
    /// `AppState::create_session` so the persisted `Session` matches the
    /// shape produced by `POST /v1/sessions`. Build structurally-equivalent
    /// inputs through both paths and assert the resulting rows are
    /// indistinguishable modulo per-row metadata (`creation_time`, `id`,
    /// `actor`). A future regression that re-introduces the direct-construct
    /// bypass will diverge on `mount_spec` / `agent_config` / `mode` and
    /// trip this test.
    #[tokio::test]
    async fn agent_queue_and_http_paths_produce_structurally_identical_sessions()
    -> anyhow::Result<()> {
        use hydra_common::api::v1 as api;

        let (handles, repo_name) = state_with_repository().await?;
        let prompt = "Fix the issue";

        // Path A — through agent_queue::spawn_for_issue.
        let (issue_id_a, _) = handles
            .store
            .add_issue(
                issue(
                    "Parity test (agent_queue path)",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item_a = handles.store.get_issue(&issue_id_a, false).await?.item;
        let result_a = queue
            .spawn_for_issue(&handles.state, &issue_id_a, &issue_item_a, &task_state)
            .await?;
        let session_id_a = result_a
            .into_session_id()
            .expect("agent_queue path should spawn");
        let session_a = handles.state.get_session(&session_id_a).await?;

        // Path B — through AppState::create_session, building the same
        // CreateSessionRequest agent_queue builds (pre-resolved bundle,
        // env_vars, secrets, etc.).
        let (issue_id_b, _) = handles
            .store
            .add_issue(
                issue(
                    "Parity test (HTTP path)",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let issue_b = handles.store.get_issue(&issue_id_b, false).await?.item;
        let session_settings_b = handles
            .state
            .apply_session_settings_defaults(issue_b.session_settings.clone());
        let repository = handles
            .state
            .repository_from_store(&repo_name)
            .await
            .expect("registered repo");
        let rev = repository
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".to_string());
        let bundle = api::sessions::Bundle::GitRepository {
            url: repository.remote_url.clone(),
            rev,
        };
        let build_cache = handles
            .state
            .config
            .build_cache
            .to_context()
            .map(|ctx| (repo_name.clone(), ctx));
        let mount_spec =
            crate::routes::sessions::mount_spec_from_create_request(bundle, build_cache);

        let mut env_vars_b = HashMap::new();
        env_vars_b.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id_b.to_string());
        // Server stamps AGENT_NAME_ENV_VAR for AgentSpec::Named.
        let request_b = api::sessions::CreateSessionRequest {
            mode: api::sessions::SessionMode::Headless,
            agent_config: api::sessions::AgentSpec::Named {
                name: hydra_common::api::v1::agents::AgentName::try_new("agent-a").unwrap(),
            },
            model: session_settings_b.model.clone(),
            mount_spec,
            image: session_settings_b.image.clone(),
            env_vars: env_vars_b,
            cpu_limit: session_settings_b.cpu_limit.clone(),
            memory_limit: session_settings_b.memory_limit.clone(),
            secrets: None,
            spawned_from: Some(issue_id_b.clone()),
            resumed_from: None,
        };
        let _ = prompt;
        let system_actor = ActorRef::System {
            worker_name: "parity_test".into(),
            on_behalf_of: None,
        };
        let (session_id_b, _) = handles
            .state
            .create_session(request_b, system_actor, issue_b.creator.clone())
            .await
            .expect("HTTP path create_session succeeds");
        let session_b = handles.state.get_session(&session_id_b).await?;

        // Structural identity — ignore per-row metadata (`creation_time`,
        // `start_time`, `end_time`, the assigned id, and `spawned_from`
        // which trivially differs because the issues are distinct).
        assert_eq!(
            session_a.mount_spec, session_b.mount_spec,
            "mount_spec must match across paths"
        );
        assert_eq!(
            session_a.agent_config, session_b.agent_config,
            "agent_config must match across paths"
        );
        assert_eq!(
            session_a.mode, session_b.mode,
            "mode must match across paths"
        );
        assert_eq!(
            session_a.image, session_b.image,
            "image must match across paths"
        );
        assert_eq!(
            session_a.cpu_limit, session_b.cpu_limit,
            "cpu_limit must match across paths"
        );
        assert_eq!(
            session_a.memory_limit, session_b.memory_limit,
            "memory_limit must match across paths"
        );
        assert_eq!(
            session_a.secrets, session_b.secrets,
            "secrets must match across paths"
        );
        assert_eq!(
            session_a.status, session_b.status,
            "status must match across paths (default Created)"
        );
        // PR-F: the transitional `Session.context` is gone; the persisted
        // `mount_spec` is the single source of truth and must match across
        // both paths.
        assert_eq!(
            session_a.mount_spec, session_b.mount_spec,
            "mount_spec must match across paths"
        );
        // Both paths inject the standard agent_queue env_vars.
        assert_eq!(
            session_a.env_vars.get(AGENT_NAME_ENV_VAR),
            session_b.env_vars.get(AGENT_NAME_ENV_VAR),
            "AGENT_NAME env_var must match"
        );

        Ok(())
    }

    // ----- Interactive branch + has_active_conversation gate -----

    /// Seed a project that declares one interactive status (`design-chat`)
    /// alongside the headless `backlog` status, and return both keys.
    async fn seed_interactive_project(
        handles: &TestStateHandles,
    ) -> (
        hydra_common::ProjectId,
        hydra_common::api::v1::projects::StatusKey,
        hydra_common::api::v1::projects::StatusKey,
    ) {
        use hydra_common::api::v1::projects::{
            Project as ApiProject, ProjectKey, StatusDefinition, StatusKey,
        };
        let interactive_key = StatusKey::try_new("design-chat").unwrap();
        let backlog_key = StatusKey::try_new("backlog").unwrap();
        let mut interactive_def = StatusDefinition::new(
            interactive_key.clone(),
            "Design Chat".to_string(),
            "#3498db".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        interactive_def.interactive = true;
        let backlog_def = StatusDefinition::new(
            backlog_key.clone(),
            "Backlog".to_string(),
            "#9b59b6".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        let project = ApiProject::new(
            ProjectKey::try_new("engineering-v2").unwrap(),
            "Engineering v2".to_string(),
            Vec::new(),
            hydra_common::api::v1::users::Username::from("alice"),
            false,
            0.0,
        );
        let (project_id, _) = handles
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();
        // Post-cutover, statuses are added independently via the
        // per-status path.
        for def in [interactive_def, backlog_def] {
            handles
                .store
                .add_status(&project_id, def, &ActorRef::test())
                .await
                .unwrap();
        }
        (project_id, interactive_key, backlog_key)
    }

    /// Build a ready issue bound to `project_id` in `status_key`, assigned
    /// to `agent-a` and using the shared test repo.
    fn interactive_issue(
        project_id: &hydra_common::ProjectId,
        status_key: &hydra_common::api::v1::projects::StatusKey,
        repo_name: &RepoName,
        description: &str,
    ) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            title: "Test Title".to_string(),
            description: description.to_string(),
            creator: default_user(),
            status: status_key.clone(),
            project_id: project_id.clone(),
            assignee: Some(test_agent_principal("agent-a")),
            session_settings: session_settings(repo_name),
            dependencies: vec![],
            patches: Vec::new(),
            archived: false,
            form: None,
            form_response: None,
        }
    }

    /// Interactive status → AgentQueue creates a Conversation (linked back
    /// via `spawned_from`), not a Session.
    #[tokio::test]
    async fn interactive_status_spawns_conversation_not_session() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(
                    &project_id,
                    &interactive_key,
                    &repo_name,
                    "interactive flow",
                ),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;

        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            result.is_spawned_conversation(),
            "expected SpawnedConversation, got something else"
        );
        let conversation_id = result.into_conversation_id().unwrap();

        // Conversation persisted with spawned_from = issue_id.
        let conversation = handles
            .state
            .store()
            .get_conversation(&conversation_id, false)
            .await?
            .item;
        assert_eq!(conversation.spawned_from, Some(issue_id.clone()));
        assert_eq!(conversation.creator.as_str(), default_user().as_str());
        assert_eq!(
            conversation.title.as_deref(),
            Some("Test Title"),
            "conversation title should mirror the issue title"
        );
        assert!(
            conversation
                .agent_name
                .as_ref()
                .is_some_and(|n| n.as_str() == "agent-a"),
            "conversation agent_name should be agent-a"
        );

        // The interactive branch goes through `create_conversation` only;
        // it does NOT also create a headless session itself.
        let task_state_after = agent_task_state(&handles.state, "agent-a").await?;
        assert!(
            !task_state_after.existing_issue_ids.contains(&issue_id),
            "interactive branch should not register a headless session for this issue \
             via AGENT_NAME env var"
        );

        Ok(())
    }

    /// Headless (non-interactive) status → unchanged regression: still
    /// goes through `create_session`, no conversation created.
    #[tokio::test]
    async fn non_interactive_status_spawns_session_no_conversation() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, _interactive, backlog_key) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &backlog_key, &repo_name, "headless flow"),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;

        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            result.is_spawned(),
            "expected a spawn (headless branch) for non-interactive status"
        );
        assert!(
            !result.is_spawned_conversation(),
            "non-interactive status must take the headless branch, not the conversation branch"
        );

        // No conversation was created for this issue.
        let q = SearchConversationsQuery {
            spawned_from: Some(issue_id.clone()),
            include_archived: Some(false),
            ..Default::default()
        };
        let convs = handles.state.store().list_conversations(&q).await?;
        assert!(
            convs.is_empty(),
            "expected zero conversations for headless issue; got {}",
            convs.len()
        );

        Ok(())
    }

    /// Live conversation linked to an interactive issue blocks any
    /// further spawn (gate applies to the interactive branch).
    #[tokio::test]
    async fn has_active_conversation_blocks_interactive_spawn() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &interactive_key, &repo_name, "guarded"),
                &ActorRef::test(),
            )
            .await?;

        // First spawn → conversation is created.
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let first = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(first.is_spawned_conversation());

        // Second invocation → blocked by has_active_conversation gate.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let second = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            !second.is_spawned(),
            "second spawn must be Skipped while a live conversation exists"
        );

        Ok(())
    }

    /// Same gate, but the live conversation is `Idle` rather than `Active`.
    /// Both pre-Closed statuses must block.
    #[tokio::test]
    async fn idle_linked_conversation_also_blocks_spawn() -> anyhow::Result<()> {
        use crate::domain::conversations::Conversation;
        use hydra_common::api::v1::agents::AgentName;
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &interactive_key, &repo_name, "idle gate"),
                &ActorRef::test(),
            )
            .await?;

        // Pre-seed an Idle conversation linked to the issue.
        let idle_conv = Conversation {
            title: None,
            agent_name: Some(AgentName::try_new("agent-a").unwrap()),
            status: ConversationStatus::Idle,
            creator: default_user(),
            session_settings: session_settings(&repo_name),
            spawned_from: Some(issue_id.clone()),
            archived: false,
        };
        handles
            .store
            .add_conversation(idle_conv, &ActorRef::test())
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            !result.is_spawned(),
            "Idle linked conversation must block further spawns"
        );

        Ok(())
    }

    /// If only Closed conversations exist for an issue, the gate is open
    /// — a fresh conversation is allowed (subject to retry budget).
    #[tokio::test]
    async fn closed_only_conversations_do_not_block_spawn() -> anyhow::Result<()> {
        use crate::domain::conversations::Conversation;
        use hydra_common::api::v1::agents::AgentName;
        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &interactive_key, &repo_name, "closed only"),
                &ActorRef::test(),
            )
            .await?;

        let closed_conv = Conversation {
            title: None,
            agent_name: Some(AgentName::try_new("agent-a").unwrap()),
            status: ConversationStatus::Closed,
            creator: default_user(),
            session_settings: session_settings(&repo_name),
            spawned_from: Some(issue_id.clone()),
            archived: false,
        };
        handles
            .store
            .add_conversation(closed_conv, &ActorRef::test())
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            result.is_spawned_conversation(),
            "Closed-only history must NOT block a fresh interactive spawn"
        );

        Ok(())
    }

    /// End-to-end (issue → conversation → session) wired through the live
    /// automation runner. AgentQueue mints a `Conversation` for the
    /// interactive issue; `SpawnConversationSessionsAutomation` reacts to
    /// `ConversationCreated` and materialises a session inheriting the
    /// `spawned_from` issue lineage and stamping both
    /// `HYDRA_ISSUE_ID` and `HYDRA_CONVERSATION_ID` env vars. The patch
    /// leg ("session produces a patch via POST /v1/patches") is already
    /// covered by the headless path's existing integration tests and
    /// reaches the same store-side reviewer/merger routing once the
    /// session is in place.
    #[tokio::test]
    async fn interactive_chain_materializes_session_inheriting_issue_lineage() -> anyhow::Result<()>
    {
        use crate::app::test_helpers::{poll_until, start_test_automation_runner};
        use hydra_common::api::v1::sessions::SearchSessionsQuery;
        use hydra_common::constants::{ENV_HYDRA_CONVERSATION_ID, ENV_HYDRA_ISSUE_ID};
        use std::time::Duration;

        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &interactive_key, &repo_name, "e2e flow"),
                &ActorRef::test(),
            )
            .await?;

        // Spawn the conversation manually (mirrors what
        // `SpawnSessionsAutomation` does in the event loop). The
        // returned ConversationCreated event flows into the live runner
        // and triggers the downstream materialization.
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;

        // Start the runner BEFORE the spawn so it subscribes to the bus
        // in time to receive the ConversationCreated event.
        let runner = start_test_automation_runner(&handles.state);

        let spawn_result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        let conversation_id = spawn_result
            .into_conversation_id()
            .expect("interactive status should mint a conversation");

        let session = poll_until(Duration::from_secs(5), || async {
            let sessions = handles
                .state
                .store()
                .list_sessions(&SearchSessionsQuery::default())
                .await
                .ok()?;
            sessions
                .into_iter()
                .filter_map(|(_, s)| {
                    (s.item.conversation_id() == Some(&conversation_id)).then_some(s)
                })
                .max_by_key(|s| s.creation_time)
        })
        .await
        .expect("expected SpawnConversationSessionsAutomation to materialize a session");
        runner.shutdown().await;

        let session = session.item;
        assert_eq!(
            session.spawned_from,
            Some(issue_id.clone()),
            "materialized session must inherit issue lineage from the conversation"
        );
        assert!(
            session.mode.greet_user(),
            "interactive-issue-backed conversation must set greet_user = true"
        );
        assert_eq!(
            session.env_vars.get(ENV_HYDRA_ISSUE_ID),
            Some(&issue_id.to_string()),
            "session must carry HYDRA_ISSUE_ID"
        );
        assert_eq!(
            session.env_vars.get(ENV_HYDRA_CONVERSATION_ID),
            Some(&conversation_id.to_string()),
            "session must carry HYDRA_CONVERSATION_ID"
        );

        Ok(())
    }

    /// Paired-mode uniformity: a fixture issue run through the headless
    /// path and through the interactive path yields sessions whose
    /// agent_config, mount_spec, image, cpu/memory limits, and secrets
    /// match. The interactive path additionally carries
    /// `HYDRA_CONVERSATION_ID` in env_vars and runs in `SessionMode::
    /// Interactive`; both paths inherit `spawned_from = Some(issue_id)`
    /// and carry `HYDRA_ISSUE_ID`. The surrounding routing pipeline
    /// (patches, child-of edges, merge policy) is keyed off these
    /// fields, so equal session content => equal downstream behaviour.
    #[tokio::test]
    async fn paired_mode_session_shapes_match_across_branches() -> anyhow::Result<()> {
        use crate::app::test_helpers::{poll_until, start_test_automation_runner};
        use hydra_common::api::v1::sessions::SearchSessionsQuery;
        use hydra_common::constants::{ENV_HYDRA_CONVERSATION_ID, ENV_HYDRA_ISSUE_ID};
        use std::time::Duration;

        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, backlog_key) = seed_interactive_project(&handles).await;

        // Headless path: backlog status (non-interactive).
        let (issue_id_a, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &backlog_key, &repo_name, "headless fixture"),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item_a = handles.store.get_issue(&issue_id_a, false).await?.item;
        let result_a = queue
            .spawn_for_issue(&handles.state, &issue_id_a, &issue_item_a, &task_state)
            .await?;
        let session_a_id = result_a
            .into_session_id()
            .expect("headless path should spawn a session");
        let session_a = handles.state.get_session(&session_a_id).await?;

        // Interactive path: design-chat status (interactive).
        let (issue_id_b, _) = handles
            .store
            .add_issue(
                interactive_issue(
                    &project_id,
                    &interactive_key,
                    &repo_name,
                    "interactive fixture",
                ),
                &ActorRef::test(),
            )
            .await?;

        let runner = start_test_automation_runner(&handles.state);
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item_b = handles.store.get_issue(&issue_id_b, false).await?.item;
        let result_b = queue
            .spawn_for_issue(&handles.state, &issue_id_b, &issue_item_b, &task_state)
            .await?;
        let conversation_id = result_b
            .into_conversation_id()
            .expect("interactive path should mint a conversation");
        let session_b_versioned = poll_until(Duration::from_secs(5), || async {
            let sessions = handles
                .state
                .store()
                .list_sessions(&SearchSessionsQuery::default())
                .await
                .ok()?;
            sessions
                .into_iter()
                .filter_map(|(_, s)| {
                    (s.item.conversation_id() == Some(&conversation_id)).then_some(s)
                })
                .max_by_key(|s| s.creation_time)
        })
        .await
        .expect("expected SpawnConversationSessionsAutomation to materialize a session");
        runner.shutdown().await;
        let session_b = session_b_versioned.item;

        // Uniformity invariants — modulo per-branch differences:
        //   * session_b.mode is Interactive (with greet_user = true) while
        //     session_a.mode is Headless — different by design.
        //   * session_b carries HYDRA_CONVERSATION_ID; session_a does not.
        //   * spawned_from on both is the SAME LOGICAL VALUE (the parent
        //     issue id), though the literal id differs because the two
        //     paths use different fixture issues.
        assert_eq!(
            session_a.agent_config, session_b.agent_config,
            "agent_config must match between headless and interactive paths"
        );
        assert_eq!(
            session_a.mount_spec, session_b.mount_spec,
            "mount_spec must match between headless and interactive paths"
        );
        assert_eq!(session_a.image, session_b.image, "image must match");
        assert_eq!(
            session_a.cpu_limit, session_b.cpu_limit,
            "cpu_limit must match"
        );
        assert_eq!(
            session_a.memory_limit, session_b.memory_limit,
            "memory_limit must match"
        );
        assert_eq!(
            session_a.secrets, session_b.secrets,
            "secrets must match (both flows go through the same defaulting)"
        );
        assert_eq!(
            session_a.spawned_from,
            Some(issue_id_a),
            "headless session must point at its source issue"
        );
        assert_eq!(
            session_b.spawned_from,
            Some(issue_id_b.clone()),
            "interactive session must inherit the source issue from the conversation"
        );
        assert_eq!(
            session_a.env_vars.get(ENV_HYDRA_ISSUE_ID),
            session_a
                .spawned_from
                .as_ref()
                .map(|id| id.to_string())
                .as_ref(),
            "headless HYDRA_ISSUE_ID must match spawned_from"
        );
        assert_eq!(
            session_b.env_vars.get(ENV_HYDRA_ISSUE_ID),
            session_b
                .spawned_from
                .as_ref()
                .map(|id| id.to_string())
                .as_ref(),
            "interactive HYDRA_ISSUE_ID must match spawned_from"
        );
        assert!(
            !session_a.env_vars.contains_key(ENV_HYDRA_CONVERSATION_ID),
            "headless session must NOT carry HYDRA_CONVERSATION_ID"
        );
        assert_eq!(
            session_b.env_vars.get(ENV_HYDRA_CONVERSATION_ID),
            Some(&conversation_id.to_string()),
            "interactive session must carry HYDRA_CONVERSATION_ID"
        );

        Ok(())
    }

    /// Retry budget bounds conversations too — after `max_tries` attempts
    /// for the same `(issue_id, status, children_snapshot)` scope key, the
    /// queue returns `RetriesExhausted`.
    #[tokio::test]
    async fn interactive_branch_respects_retry_budget() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (project_id, interactive_key, _backlog) = seed_interactive_project(&handles).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                interactive_issue(&project_id, &interactive_key, &repo_name, "retry budget"),
                &ActorRef::test(),
            )
            .await?;

        // Attempt 1 of 1 — succeeds.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let first = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        let conversation_id = first.into_conversation_id().unwrap();

        // Close the seeded conversation so the active-conversation gate is
        // open again, leaving only the retry budget to enforce.
        handles
            .state
            .close_conversation(&conversation_id, ActorRef::test())
            .await
            .map_err(anyhow::Error::from)?;

        // Attempt 2 — exhausted: budget reaches its cap before we hit the
        // dispatch.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            matches!(result, SpawnResult::RetriesExhausted { .. }),
            "expected RetriesExhausted on the second interactive attempt"
        );

        Ok(())
    }

    /// Set `max_simultaneous_sessions = Some(cap)` on the default-project
    /// `open` status while preserving every other field. Goes through
    /// `update_status` for the same reason as
    /// `enable_auto_archive` in `auto_archive.rs`.
    async fn set_status_cap(handles: &TestStateHandles, status_key: &str, cap: Option<u32>) {
        use crate::domain::projects::default_project_id;
        let project_id = default_project_id();
        let versioned = handles
            .store
            .get_project(&project_id, false)
            .await
            .expect("project must exist");
        let mut status = versioned
            .item
            .statuses
            .iter()
            .find(|s| s.key.as_str() == status_key)
            .cloned()
            .unwrap_or_else(|| panic!("status '{status_key}' not found on default project"));
        status.max_simultaneous_sessions = cap;
        let key = status.key.clone();
        handles
            .store
            .update_status(&project_id, &key, status, &ActorRef::test())
            .await
            .expect("update_status failed");
    }

    /// Per-status cap defers spawns once the count of active sessions
    /// (across all agents in the status) reaches the cap; the spawn
    /// resumes after one of the active sessions terminates.
    #[tokio::test]
    async fn status_cap_defers_spawns_at_capacity_then_resumes_when_session_terminates()
    -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        set_status_cap(&handles, "open", Some(1)).await;

        // First ready issue: spawn succeeds and consumes the cap.
        let (first_id, _) = handles
            .store
            .add_issue(
                issue("first", status("open"), Some("agent-a"), vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let first_issue = handles.store.get_issue(&first_id, false).await?.item;
        let first_result = queue
            .spawn_for_issue(&handles.state, &first_id, &first_issue, &task_state)
            .await?;
        let first_session = first_result
            .into_session_id()
            .expect("first issue must spawn under fresh status cap");

        // Second ready issue: deferred — cap of 1 is already saturated.
        let (second_id, _) = handles
            .store
            .add_issue(
                issue(
                    "second",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let second_issue = handles.store.get_issue(&second_id, false).await?.item;
        let deferred = queue
            .spawn_for_issue(&handles.state, &second_id, &second_issue, &task_state)
            .await?;
        assert!(
            matches!(deferred, SpawnResult::Skipped),
            "second issue must defer while status cap is saturated"
        );

        // Terminate the first session; the cap should now allow the
        // second issue's spawn to proceed.
        record_completed_task(&handles, first_session).await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let second_issue = handles.store.get_issue(&second_id, false).await?.item;
        let retried = queue
            .spawn_for_issue(&handles.state, &second_id, &second_issue, &task_state)
            .await?;
        assert!(
            retried.is_spawned(),
            "second issue must spawn once cap is freed"
        );

        Ok(())
    }

    /// Cap counts interactive (conversation-backed) sessions alongside
    /// headless ones: one of each saturates a cap of 2 and a third
    /// (headless) spawn is deferred. The interactive path is exercised
    /// via the `interactive` status on a separate project, but its
    /// session lands against the default-project `open` cap when the
    /// issue spans that status (modeled here by adding an extra agent
    /// task and an extra `add_session` for the interactive count).
    #[tokio::test]
    async fn status_cap_counts_both_interactive_and_headless_sessions() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        set_status_cap(&handles, "open", Some(2)).await;

        // Issue A: headless spawn (counts 1 toward cap).
        let (a_id, _) = handles
            .store
            .add_issue(
                issue("A", status("open"), Some("agent-a"), vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let a_issue = handles.store.get_issue(&a_id, false).await?.item;
        let _ = queue
            .spawn_for_issue(&handles.state, &a_id, &a_issue, &task_state)
            .await?
            .into_session_id()
            .expect("A must spawn (headless)");

        // Issue B: pre-seed an interactive (conversation-backed) session
        // whose `spawned_from = B` so it counts toward the per-status
        // cap alongside the headless spawn for A.
        let (b_id, _) = handles
            .store
            .add_issue(
                issue("B", status("open"), Some("agent-b"), vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;
        let conv_id = ConversationId::new();
        let interactive_session = {
            use crate::domain::sessions::{AgentConfig, SessionMode};
            Session::new(
                Username::from("creator"),
                Some(b_id.clone()),
                None,
                AgentConfig::new(None, None, Some("interactive".into()), None),
                mount_spec_from_create_request(Bundle::None, None),
                None,
                HashMap::new(),
                None,
                None,
                None,
                SessionMode::Interactive {
                    conversation_id: conv_id,
                    idle_timeout: None,
                    greet_user: false,
                },
                Status::Running,
                None,
                None,
            )
        };
        handles
            .state
            .add_session(interactive_session, Utc::now(), ActorRef::test())
            .await?;

        // Cap = 2; both slots taken. Issue C must defer.
        let (c_id, _) = handles
            .store
            .add_issue(
                issue("C", status("open"), Some("agent-a"), vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let c_issue = handles.store.get_issue(&c_id, false).await?.item;
        let deferred = queue
            .spawn_for_issue(&handles.state, &c_id, &c_issue, &task_state)
            .await?;
        assert!(
            matches!(deferred, SpawnResult::Skipped),
            "C must defer with cap saturated by headless + interactive sessions"
        );

        Ok(())
    }

    /// Per-status cap and per-agent cap are independent; whichever is
    /// tighter blocks. Here the agent cap allows the spawn (default,
    /// large) but the status cap is set to 0 so no spawn proceeds.
    #[tokio::test]
    async fn status_cap_blocks_when_agent_cap_permits() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        set_status_cap(&handles, "open", Some(0)).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "blocked-by-status",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        // Agent cap is large (default), so the only block source is the
        // status cap of 0.
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            matches!(result, SpawnResult::Skipped),
            "status cap of 0 must block even when agent cap is loose"
        );

        Ok(())
    }

    /// Inverse: agent cap is the tighter bound; status cap is loose
    /// (None) or large enough that it doesn't kick in. The agent cap
    /// continues to be enforced regardless of the status cap.
    #[tokio::test]
    async fn agent_cap_blocks_when_status_cap_permits() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        // Status cap: loose (large).
        set_status_cap(&handles, "open", Some(100)).await;

        // Agent cap: tight (1). Saturate it with an unrelated active
        // session so the agent at_capacity branch trips first.
        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous_headless = 1;
        let saturating_session = task(
            "saturating",
            Bundle::None,
            None,
            None,
            HashMap::from([
                (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                (ISSUE_ID_ENV_VAR.to_string(), "i-saturat".to_string()),
            ]),
        );
        let mut running_session = saturating_session;
        running_session.status = Status::Running;
        handles
            .state
            .add_session(running_session, Utc::now(), ActorRef::test())
            .await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "blocked-by-agent",
                    status("open"),
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            matches!(result, SpawnResult::Skipped),
            "agent cap saturation must block even when status cap is loose"
        );

        Ok(())
    }

    /// Overlay a `cpu_limit` / `memory_limit` override on the default
    /// project's named status by re-running `update_status` with the
    /// existing definition plus the requested session-settings override.
    async fn set_status_session_settings(
        handles: &TestStateHandles,
        status_key: &str,
        cpu_limit: Option<&str>,
        memory_limit: Option<&str>,
    ) {
        let project_id = crate::domain::projects::default_project_id();
        let key = StatusKey::try_new(status_key).expect("valid status key");
        let project = handles
            .store
            .get_project(&project_id, false)
            .await
            .expect("default project exists")
            .item;
        let mut def = project
            .statuses
            .iter()
            .find(|s| s.key == key)
            .expect("status exists on default project")
            .clone();
        def.session_settings.cpu_limit = cpu_limit.map(str::to_string);
        def.session_settings.memory_limit = memory_limit.map(str::to_string);
        handles
            .state
            .update_status(&project_id, &key, def, &ActorRef::test())
            .await
            .expect("update_status succeeds");
    }

    /// PR-1 anchor: a status with `session_settings.cpu_limit = "500m"`
    /// flows through to the spawned session's `cpu_limit` when the issue
    /// itself sets no override. The headless branch is the simpler of the
    /// two spawn paths and exercises the merge precedence end-to-end:
    /// status override → spawn-time `Session.cpu_limit`.
    #[tokio::test]
    async fn status_level_cpu_limit_flows_to_spawned_session() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        set_status_session_settings(&handles, "open", Some("500m"), Some("256Mi")).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Inherit from status".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        image: Some("repo-image".to_string()),
                        ..SessionSettings::default()
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        let session_id = result.into_session_id().expect("session spawned");
        let session = handles.state.get_session(&session_id).await?;
        assert_eq!(
            session.cpu_limit.as_deref(),
            Some("500m"),
            "status-level cpu_limit must propagate to the spawned session"
        );
        assert_eq!(
            session.memory_limit.as_deref(),
            Some("256Mi"),
            "status-level memory_limit must propagate to the spawned session"
        );

        Ok(())
    }

    /// Overlay a `SessionSettings` blob on the default project's
    /// project-level record. Mirrors [`set_status_session_settings`]
    /// for the project layer.
    async fn set_project_session_settings(
        handles: &TestStateHandles,
        session_settings: hydra_common::api::v1::issues::SessionSettings,
    ) {
        let project_id = crate::domain::projects::default_project_id();
        let mut project = handles
            .store
            .get_project(&project_id, false)
            .await
            .expect("default project exists")
            .item;
        project.session_settings = session_settings;
        handles
            .store
            .update_project(&project_id, project, &ActorRef::test())
            .await
            .expect("update_project succeeds");
    }

    /// Project-level `image` flows through to the spawned session when
    /// neither the issue nor the status sets it. Anchors the new
    /// project layer in the merge chain.
    #[tokio::test]
    async fn project_level_image_flows_to_spawned_session() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let mut project_settings = hydra_common::api::v1::issues::SessionSettings::default();
        project_settings.image = Some("project-image".to_string());
        set_project_session_settings(&handles, project_settings).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Inherit image from project".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        ..SessionSettings::default()
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        let session_id = result.into_session_id().expect("session spawned");
        let session = handles.state.get_session(&session_id).await?;
        assert_eq!(
            session.image.as_deref(),
            Some("project-image"),
            "project-level image must propagate when neither issue nor status overrides it"
        );

        Ok(())
    }

    /// Locks the full precedence chain: issue beats status beats project
    /// beats default. Each layer sets a different field; the resolved
    /// settings must combine the per-field winners.
    #[tokio::test]
    async fn resolve_session_settings_merges_issue_status_project_layers() -> anyhow::Result<()> {
        let (handles, _repo_name) = state_with_repository().await?;

        // Project layer: image=A, cpu=A, memory=A
        let mut project_settings = hydra_common::api::v1::issues::SessionSettings::default();
        project_settings.image = Some("project-image".to_string());
        project_settings.cpu_limit = Some("100m".to_string());
        project_settings.memory_limit = Some("128Mi".to_string());
        set_project_session_settings(&handles, project_settings).await;

        // Status layer: image=B, cpu=B (memory unset — falls through)
        let project_id = crate::domain::projects::default_project_id();
        let key = StatusKey::try_new("open").expect("status key");
        let project = handles.store.get_project(&project_id, false).await?.item;
        let mut def = project
            .statuses
            .iter()
            .find(|s| s.key == key)
            .expect("open status exists")
            .clone();
        def.session_settings.image = Some("status-image".to_string());
        def.session_settings.cpu_limit = Some("200m".to_string());
        handles
            .state
            .update_status(&project_id, &key, def, &ActorRef::test())
            .await?;

        // Issue layer: image=C (cpu and memory unset — fall through)
        let issue = Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: "merge precedence".to_string(),
            creator: default_user(),
            status: status("open"),
            project_id,
            assignee: Some(test_agent_principal("agent-a")),
            session_settings: SessionSettings {
                image: Some("issue-image".to_string()),
                ..SessionSettings::default()
            },
            dependencies: vec![],
            patches: Vec::new(),
            archived: false,
            form: None,
            form_response: None,
        };

        let resolved = resolve_session_settings_for_issue(&handles.state, &issue).await;
        assert_eq!(
            resolved.image.as_deref(),
            Some("issue-image"),
            "issue image must beat status image and project image"
        );
        assert_eq!(
            resolved.cpu_limit.as_deref(),
            Some("200m"),
            "status cpu_limit must beat project cpu_limit when issue is unset"
        );
        assert_eq!(
            resolved.memory_limit.as_deref(),
            Some("128Mi"),
            "project memory_limit must surface when both issue and status are unset"
        );

        Ok(())
    }

    /// PR-1 anchor: when the issue and its resolved status both set a
    /// `cpu_limit`, the issue's value wins. Locks the merge order so
    /// future per-status `SessionSettings` additions can't accidentally
    /// flip the precedence.
    #[tokio::test]
    async fn issue_cpu_limit_wins_over_status_cpu_limit() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        set_status_session_settings(&handles, "open", Some("500m"), None).await;

        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Issue overrides status".to_string(),
                    creator: default_user(),
                    status: status("open"),
                    project_id: crate::domain::projects::default_project_id(),
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings {
                        repo_name: Some(repo_name.clone()),
                        image: Some("repo-image".to_string()),
                        cpu_limit: Some("1000m".to_string()),
                        ..SessionSettings::default()
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    archived: false,
                    form: None,
                    form_response: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        let session_id = result.into_session_id().expect("session spawned");
        let session = handles.state.get_session(&session_id).await?;
        assert_eq!(
            session.cpu_limit.as_deref(),
            Some("1000m"),
            "issue-level cpu_limit must win over the status-level override"
        );

        Ok(())
    }
}
