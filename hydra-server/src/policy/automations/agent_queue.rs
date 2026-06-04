#[cfg(test)]
use crate::domain::issues::IssueStatus;
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
        issues::{Issue, IssueDependencyType},
    },
    store::{Status, StoreError},
};
use anyhow::Context;
#[cfg(test)]
use hydra_common::RepoName;
use hydra_common::api::v1 as api;
use hydra_common::api::v1::projects::StatusKey;
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::{IssueId, SessionId, VersionNumber};
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
    /// The issue was skipped (not eligible for spawning).
    Skipped,
    /// The spawn attempt retry cap has been exhausted for this issue.
    RetriesExhausted { issue_id: IssueId, max_tries: i32 },
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

    /// Returns `true` if a session was spawned.
    fn is_spawned(&self) -> bool {
        matches!(self, SpawnResult::Spawned(_))
    }
}

#[derive(Clone, Debug)]
pub struct SpawnAttempt {
    status: StatusKey,
    attempts: i32,
    children_snapshot: HashMap<IssueId, VersionNumber>,
    feedback: Option<String>,
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

    async fn build_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> anyhow::Result<Option<SessionId>> {
        let session_settings =
            state.apply_session_settings_defaults(issue.session_settings.clone());

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

    async fn register_spawn_attempt(
        &self,
        issue_id: &IssueId,
        status: StatusKey,
        children_snapshot: HashMap<IssueId, VersionNumber>,
        feedback: Option<String>,
        max_tries: i32,
    ) -> bool {
        let mut attempts = self.spawn_attempts.write().await;
        let entry = attempts.entry(issue_id.clone()).or_insert(SpawnAttempt {
            status: status.clone(),
            attempts: 0,
            children_snapshot: HashMap::new(),
            feedback: None,
        });

        let status_changed = entry.status != status;
        let children_changed = entry.children_snapshot != children_snapshot;
        let feedback_changed = entry.feedback != feedback;

        if status_changed || children_changed || feedback_changed {
            *entry = SpawnAttempt {
                status,
                attempts: 0,
                children_snapshot,
                feedback,
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
        let has_feedback = issue.feedback.is_some();

        // Assignment check: compare against the typed
        // `Principal::Agent { name }` — bare-string matching is gone.
        // Issues assigned to a `Principal::User { name == agent.name }`
        // (the typo direction) are deliberately NOT picked up.
        // Unassigned issues are NOT picked up here; the `spawn_sessions`
        // automation auto-assigns them to the configured assignment agent
        // and persists the update before this method is called.
        let is_assignment_match = matches!(
            issue.assignee.as_ref(),
            Some(hydra_common::principal::Principal::Agent { name })
                if name.as_str() == self.agent.name
        );
        if !is_assignment_match {
            return Ok(SpawnResult::Skipped);
        }

        // Compute guard conditions via project-aware status resolution.
        // The terminal predicate is `unblocks_parents` — every status that
        // halted spawn in the legacy enum world declared
        // `unblocks_parents = true` on `DefaultProject`. The "hard skip"
        // for `dropped` (work explicitly abandoned, never re-spawned even
        // on feedback) is derived from `cascades_to_children && !unblocks_dependents`:
        // those are the same flags `dropped` carries on `DefaultProject`
        // and the same flags any custom drop-like status will carry.
        let resolved = state.resolve_status(issue).await.with_context(|| {
            format!("failed to resolve status for issue {issue_id} during spawn check")
        })?;
        let is_terminal = resolved.unblocks_parents;
        let is_dropped = resolved.cascades_to_children && !resolved.unblocks_dependents;
        let is_ready = state
            .is_issue_ready(issue_id)
            .await
            .context("failed to determine if issue is ready")?;
        let active_tasks = task_state.running_tasks + task_state.pending_tasks;
        let max_simultaneous = self.agent.max_simultaneous as usize;
        let at_capacity = max_simultaneous == 0 || active_tasks >= max_simultaneous;
        let has_active_session = task_state.existing_issue_ids.contains(issue_id);
        let parent_running = parent_has_running_task(state, issue).await?;

        // Determine whether to skip this issue.
        // Feedback bypasses terminal status (Closed/Failed), dependency readiness, and
        // parent running checks. Dropped is a hard skip — feedback never re-spawns a
        // dropped issue, since "dropped" means the work was explicitly abandoned.
        // Active session and capacity checks are always enforced.
        if at_capacity
            || has_active_session
            || is_dropped
            || (!has_feedback && (is_terminal || !is_ready || parent_running))
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
            .register_spawn_attempt(
                issue_id,
                issue.status.clone(),
                children_snapshot,
                issue.feedback.clone(),
                max_tries,
            )
            .await
        {
            return Ok(SpawnResult::RetriesExhausted {
                issue_id: issue_id.clone(),
                max_tries,
            });
        }

        let maybe_session_id = self.build_task(state, issue_id, issue).await?;
        let Some(session_id) = maybe_session_id else {
            return Ok(SpawnResult::Skipped);
        };

        Ok(SpawnResult::Spawned(session_id))
    }
}

pub(crate) struct AgentTaskState {
    pub(crate) existing_issue_ids: HashSet<IssueId>,
    pub(crate) running_tasks: usize,
    pub(crate) pending_tasks: usize,
}

pub(crate) async fn agent_task_state(
    state: &AppState,
    agent_name: &str,
) -> Result<AgentTaskState, StoreError> {
    let mut task_state = AgentTaskState {
        existing_issue_ids: HashSet::new(),
        running_tasks: 0,
        pending_tasks: 0,
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

        match session.status {
            Status::Created => task_state.pending_tasks += 1,
            Status::Pending | Status::Running => task_state.running_tasks += 1,
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
            if matches!(
                state.get_session(&task_id).await?.status,
                Status::Pending | Status::Running
            ) {
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
            false,
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
                false,
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
                false,
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
            deleted: false,
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
            Some("repo-image".to_string()),
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
        status: IssueStatus,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        repo_name: &RepoName,
    ) -> Issue {
        Issue::new(
            issue_type,
            "Test Title".to_string(),
            description.to_string(),
            default_user(),
            String::new(),
            status.into(),
            assignee.map(test_agent_principal),
            Some(session_settings(repo_name)),
            dependencies,
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn issue(
        description: &str,
        status: IssueStatus,
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

    fn issue_without_repo(description: &str, status: IssueStatus, assignee: Option<&str>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            title: String::new(),
            description: description.to_string(),
            creator: default_user(),
            progress: String::new(),
            status: status.into(),
            project_id: None,
            assignee: assignee.map(test_agent_principal),
            session_settings: SessionSettings::default(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
            form: None,
            form_response: None,
            feedback: None,
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
                    IssueStatus::Open,
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
                    IssueStatus::InProgress,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        handles
            .store
            .add_issue(
                issue(
                    "Ignore closed",
                    IssueStatus::Closed,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
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
                    IssueStatus::Open,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: vec![patch_id.clone()],
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    IssueStatus::Open,
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
    async fn assignment_agent_queue_skips_unassigned_issue() -> anyhow::Result<()> {
        // The queue no longer special-cases the assignment agent.
        // Auto-assignment is the `spawn_sessions` automation's job; once
        // that runs, the issue is explicitly assigned and reaches the
        // queue via the normal `is_assignment_match` path.
        let handles = test_state_handles();
        seed_agent_prompt(&handles, "assignment", "Assign unowned issues").await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue_without_repo("Needs assignment", IssueStatus::Open, None),
                &ActorRef::test(),
            )
            .await?;

        let q = {
            use crate::domain::agents::Agent;
            AgentQueue::new(
                Agent::new(
                    "assignment".to_string(),
                    "/agents/assignment/prompt.md".to_string(),
                    None,
                    DEFAULT_AGENT_MAX_TRIES,
                    DEFAULT_AGENT_MAX_SIMULTANEOUS,
                    true,
                    false,
                    Vec::new(),
                ),
                shared_attempts(),
            )
        };
        let task_state = agent_task_state(&handles.state, "assignment").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = q
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
                issue_without_repo("Needs assignment", IssueStatus::Open, None),
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
                issue("Blocker", IssueStatus::Open, None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Blocked issue",
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
    async fn does_not_spawn_when_at_max_simultaneous() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_simultaneous = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Already running".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
        queue.agent.max_simultaneous = 2;

        let (handles, repo_name) = state_with_repository().await?;
        let (first_issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "First issue".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
        issue_item.status = IssueStatus::InProgress.into();
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
                    IssueStatus::Open,
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
                    IssueStatus::Closed,
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
                    IssueStatus::Open,
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
                    IssueStatus::Closed,
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

        // Update the child's progress field — status stays Closed but
        // the version bumps, which counts as parent progress.
        let child = handles.store.get_issue(&child_id, false).await?;
        let mut child_item = child.item;
        child_item.progress = "made further notes".to_string();
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
                    IssueStatus::Open,
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
                    IssueStatus::Closed,
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
            10,
            true,
            false,
            Vec::new(),
        );

        let queue = AgentQueue::new(agent, shared_attempts());

        assert_eq!(queue.agent.name, "test-agent");
        assert_eq!(queue.agent.prompt_path, "/agents/test-agent/prompt.md");
        assert_eq!(queue.agent.max_tries, 5);
        assert_eq!(queue.agent.max_simultaneous, 10);
        assert!(queue.agent.is_assignment_agent);
    }

    #[tokio::test]
    async fn does_not_spawn_for_dropped_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Dropped issue",
                    IssueStatus::Dropped,
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
        assert!(!result.is_spawned());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_for_failed_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Failed issue",
                    IssueStatus::Failed,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
        // before calling `create_session`; `resolved.context.bundle` then
        // mirrors that.
        let _ = repo_name.clone();
        assert_eq!(
            resolved.context.bundle,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
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
                    },
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
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
                    IssueStatus::Open,
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
            IssueStatus::Open,
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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
                false,
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
            deleted: false,
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
            false,
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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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

    #[tokio::test]
    async fn feedback_bypasses_guards() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        // Create a blocker issue (open, so the dependent is not ready).
        let (blocker_id, _) = handles
            .store
            .add_issue(
                issue("Blocker", IssueStatus::Open, None, vec![], &repo_name),
                &ActorRef::test(),
            )
            .await?;

        // Create a parent issue with a running session (blocks child spawning).
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Parent issue",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let (parent_task_id, _) = handles
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
            .transition_task_to_pending(&parent_task_id, ActorRef::test())
            .await?;

        // Create a closed issue with feedback, blocked deps, and a running parent.
        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Needs feedback".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Closed.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![
                        IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone()),
                        IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
                    ],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: None,
                },
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");

        // Without feedback: should NOT spawn (closed + blocked deps + parent running).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned(), "should not spawn without feedback");

        // Set feedback on the issue.
        let mut updated_issue = handles.store.get_issue(&issue_id, false).await?.item;
        updated_issue.feedback = Some("Please fix the approach".to_string());
        handles
            .store
            .update_issue(&issue_id, updated_issue, &ActorRef::test())
            .await?;

        // With feedback: should spawn despite closed status, blocked deps, and parent running.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned(), "should spawn with feedback set");

        // Feedback does NOT bypass the active session guard.
        // Simulate an active session for this issue by adding it to existing_issue_ids.
        let mut task_state = agent_task_state(&handles.state, "agent-a").await?;
        task_state.existing_issue_ids.insert(issue_id.clone());
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(
            !result.is_spawned(),
            "feedback should NOT bypass active session guard"
        );

        Ok(())
    }

    #[tokio::test]
    async fn feedback_does_not_bypass_dropped_status() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Dropped with feedback".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Dropped.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: session_settings(&repo_name),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: Some("please reconsider".to_string()),
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
        assert!(
            !result.is_spawned(),
            "feedback should NOT bypass Dropped status"
        );

        Ok(())
    }

    #[tokio::test]
    async fn feedback_change_resets_spawn_attempts() -> anyhow::Result<()> {
        let handles = test_state_handles();
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;

        let (issue_id, _) = handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Feedback attempt reset".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open.into(),
                    project_id: None,
                    assignee: Some(test_agent_principal("agent-a")),
                    session_settings: SessionSettings::default(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                    form: None,
                    form_response: None,
                    feedback: Some("first feedback".to_string()),
                },
                &ActorRef::test(),
            )
            .await?;

        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        // First spawn attempt with feedback should succeed.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned(), "first attempt should succeed");
        record_completed_task(&handles, result.into_session_id().unwrap()).await?;

        // Second attempt with same feedback should be blocked (max_tries=1).
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(!result.is_spawned(), "should be blocked by max_tries");

        // Change the feedback text.
        let mut updated_issue = handles.store.get_issue(&issue_id, false).await?.item;
        updated_issue.feedback = Some("new feedback".to_string());
        handles
            .store
            .update_issue(&issue_id, updated_issue, &ActorRef::test())
            .await?;

        // Should spawn again because feedback changed resets attempts.
        let task_state = agent_task_state(&handles.state, "agent-a").await?;
        let issue_item = handles.store.get_issue(&issue_id, false).await?.item;
        let result = queue
            .spawn_for_issue(&handles.state, &issue_id, &issue_item, &task_state)
            .await?;
        assert!(result.is_spawned(), "new feedback should reset attempts");

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
                    IssueStatus::Open,
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
                    IssueStatus::Open,
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
}
