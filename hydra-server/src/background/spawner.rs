#[cfg(test)]
use crate::domain::actors::ActorId;
#[cfg(test)]
use crate::domain::issues::{IssueDependency, IssueType};
#[cfg(test)]
use crate::domain::users::Username;
use crate::{
    app::AppState,
    domain::{
        issues::{Issue, IssueDependencyType, IssueStatus},
        sessions::BundleSpec,
    },
    store::{Session, Status, StoreError},
};
use anyhow::Context;
use async_trait::async_trait;
#[cfg(test)]
use hydra_common::RepoName;
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::{IssueId, VersionNumber};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared spawn attempt state that persists across scheduler iterations.
pub type SharedSpawnAttempts = Arc<RwLock<HashMap<IssueId, SpawnAttempt>>>;

pub const ISSUE_ID_ENV_VAR: &str = "HYDRA_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "HYDRA_AGENT_NAME";

#[async_trait]
pub trait Spawner: Send + Sync {
    fn name(&self) -> &str;
    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Session>>;
}

#[derive(Clone, Debug)]
pub struct SpawnAttempt {
    status: IssueStatus,
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

    async fn build_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
        prompt: &str,
    ) -> anyhow::Result<Option<Session>> {
        let session_settings =
            state.apply_session_settings_defaults(issue.session_settings.clone());
        let bundle = match (
            session_settings.remote_url.as_ref(),
            session_settings.repo_name.as_ref(),
        ) {
            (Some(remote_url), _) if !remote_url.trim().is_empty() => {
                let rev = session_settings
                    .branch
                    .clone()
                    .unwrap_or_else(|| "main".to_string());
                BundleSpec::GitRepository {
                    url: remote_url.trim().to_string(),
                    rev,
                }
            }
            (_, Some(repo_name)) => {
                let repository = state
                    .repository_from_store(repo_name)
                    .await
                    .context("failed to load repository for issue task")?;
                let rev = session_settings
                    .branch
                    .clone()
                    .or_else(|| repository.default_branch.clone());

                BundleSpec::ServiceRepository {
                    name: repo_name.clone(),
                    rev,
                }
            }
            _ => BundleSpec::None,
        };

        let image = session_settings
            .image
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let mut env_vars = HashMap::new();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string());
        env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), self.agent.name.clone());

        // Merge agent-level secrets with issue-level secrets, deduplicating.
        let merged_secrets = {
            let mut seen = HashSet::new();
            let mut secrets = Vec::new();
            for s in self
                .agent
                .secrets
                .iter()
                .chain(session_settings.secrets.iter().flatten())
            {
                if seen.insert(s.clone()) {
                    secrets.push(s.clone());
                }
            }
            if secrets.is_empty() {
                None
            } else {
                Some(secrets)
            }
        };

        Ok(Some(Session::new(
            prompt.to_string(),
            bundle,
            Some(issue_id.clone()),
            issue.creator.clone(),
            image,
            session_settings.model.clone(),
            env_vars,
            session_settings.cpu_limit.clone(),
            session_settings.memory_limit.clone(),
            merged_secrets,
            Status::Created,
            None,
            None,
        )))
    }

    async fn register_spawn_attempt(
        &self,
        issue_id: &IssueId,
        status: IssueStatus,
        children_snapshot: HashMap<IssueId, VersionNumber>,
        max_tries: i32,
    ) -> bool {
        let mut attempts = self.spawn_attempts.write().await;
        let entry = attempts.entry(issue_id.clone()).or_insert(SpawnAttempt {
            status,
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
    /// bulk `spawn()` method and is intended for event-driven automations that
    /// already know which issue to evaluate.
    pub(crate) async fn spawn_for_issue(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
        task_state: &AgentTaskState,
        cached_prompt: &mut Option<String>,
    ) -> anyhow::Result<Option<Session>> {
        // Assignment check.
        if self.agent.is_assignment_agent {
            if let Some(name) = &issue.assignee {
                if name != &self.agent.name {
                    return Ok(None);
                }
            }
        } else if issue.assignee.as_deref() != Some(self.agent.name.as_str()) {
            return Ok(None);
        }

        // Skip terminal issues.
        if matches!(
            issue.status,
            IssueStatus::Closed
                | IssueStatus::Dropped
                | IssueStatus::Rejected
                | IssueStatus::Failed
        ) {
            return Ok(None);
        }

        // Dependency readiness.
        let is_ready = state
            .is_issue_ready(issue_id)
            .await
            .context("failed to determine if issue is ready")?;
        if !is_ready {
            return Ok(None);
        }

        // Capacity check.
        let active_tasks = task_state.running_tasks + task_state.pending_tasks;
        let max_simultaneous = self.agent.max_simultaneous as usize;
        if max_simultaneous == 0 || active_tasks >= max_simultaneous {
            return Ok(None);
        }

        // Already has an active session.
        if task_state.existing_issue_ids.contains(issue_id) {
            return Ok(None);
        }

        // Parent has a running task.
        if parent_has_running_task(state, issue).await? {
            return Ok(None);
        }

        // Resolve prompt (lazily cached across calls).
        if cached_prompt.is_none() {
            *cached_prompt = Some(
                state
                    .resolve_agent_prompt(&self.agent.prompt_path)
                    .await
                    .with_context(|| {
                        format!(
                            "failed to fetch prompt for agent '{}' at path '{}'",
                            self.agent.name, self.agent.prompt_path
                        )
                    })?,
            );
        }
        let prompt = cached_prompt.as_deref().unwrap();

        let maybe_task = self.build_task(state, issue_id, issue, prompt).await?;
        let Some(task) = maybe_task else {
            return Ok(None);
        };

        // Spawn attempt tracking.
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
            .register_spawn_attempt(issue_id, issue.status, children_snapshot, max_tries)
            .await
        {
            return Ok(None);
        }

        Ok(Some(task))
    }
}

#[async_trait]
impl Spawner for AgentQueue {
    fn name(&self) -> &str {
        &self.agent.name
    }

    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Session>> {
        let task_state = agent_task_state(state, &self.agent.name)
            .await
            .context("failed to list tasks for agent queue")?;

        let max_simultaneous = self.agent.max_simultaneous as usize;
        if max_simultaneous == 0 {
            return Ok(Vec::new());
        }

        let active_tasks = task_state.running_tasks + task_state.pending_tasks;
        if active_tasks >= max_simultaneous {
            return Ok(Vec::new());
        }

        let mut remaining_capacity = max_simultaneous - active_tasks;

        let is_assignment_agent = self.agent.is_assignment_agent;

        let issues = state
            .list_issues()
            .await
            .context("failed to list issues for agent queue")?;

        let mut cached_prompt: Option<String> = None;

        let mut tasks = Vec::new();
        for (issue_id, issue) in issues {
            let issue = issue.item;
            if is_assignment_agent {
                if let Some(name) = &issue.assignee
                    && name != &self.agent.name
                {
                    continue;
                }
            } else if issue.assignee.as_deref() != Some(self.agent.name.as_str()) {
                continue;
            }

            // Do not spawn tasks for terminal issues.
            if matches!(
                issue.status,
                IssueStatus::Closed
                    | IssueStatus::Dropped
                    | IssueStatus::Rejected
                    | IssueStatus::Failed
            ) {
                continue;
            }

            let is_ready = state
                .is_issue_ready(&issue_id)
                .await
                .context("failed to determine if issue is ready")?;
            if !is_ready {
                continue;
            }

            if remaining_capacity == 0 {
                break;
            }

            if task_state.existing_issue_ids.contains(&issue_id) {
                continue;
            }

            if parent_has_running_task(state, &issue).await? {
                continue;
            }

            if cached_prompt.is_none() {
                cached_prompt = Some(
                    state
                        .resolve_agent_prompt(&self.agent.prompt_path)
                        .await
                        .with_context(|| {
                            format!(
                                "failed to fetch prompt for agent '{}' at path '{}'",
                                self.agent.name, self.agent.prompt_path
                            )
                        })?,
                );
            }
            let prompt = cached_prompt.as_deref().unwrap();

            let maybe_task = self.build_task(state, &issue_id, &issue, prompt).await?;
            let Some(task) = maybe_task else {
                continue;
            };

            let max_tries = self.max_tries_for_issue(&issue);
            let children_snapshot = {
                let child_ids = state
                    .get_issue_children(&issue_id)
                    .await
                    .unwrap_or_default();
                let mut snapshot = HashMap::new();
                for child_id in child_ids {
                    if let Ok(child) = state.get_issue(&child_id, false).await {
                        snapshot.insert(child_id, child.version);
                    }
                }
                snapshot
            };
            if !self
                .register_spawn_attempt(&issue_id, issue.status, children_snapshot, max_tries)
                .await
            {
                continue;
            }

            tasks.push(task);
            remaining_capacity -= 1;
        }

        Ok(tasks)
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
    use crate::domain::messages::Message;
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::domain::sessions::{Bundle, BundleSpec};
    use crate::{
        app::Repository,
        config::{DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
        test::{TestStateHandles, test_state_handles, test_state_with_repo_handles},
    };
    use chrono::Utc;

    fn default_user() -> Username {
        Username::from("spawner")
    }

    fn shared_attempts() -> SharedSpawnAttempts {
        Arc::new(RwLock::new(HashMap::new()))
    }

    fn queue(agent_name: &str) -> AgentQueue {
        queue_with_attempts(agent_name, shared_attempts())
    }

    fn queue_with_attempts(agent_name: &str, attempts: SharedSpawnAttempts) -> AgentQueue {
        use crate::domain::agents::Agent;
        AgentQueue::new(
            Agent::new(
                agent_name.to_string(),
                format!("/agents/{agent_name}/prompt.md"),
                DEFAULT_AGENT_MAX_TRIES,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                false,
                Vec::new(),
            ),
            attempts,
        )
    }

    fn queue_with_secrets(agent_name: &str, secrets: Vec<String>) -> AgentQueue {
        use crate::domain::agents::Agent;
        AgentQueue::new(
            Agent::new(
                agent_name.to_string(),
                format!("/agents/{agent_name}/prompt.md"),
                DEFAULT_AGENT_MAX_TRIES,
                DEFAULT_AGENT_MAX_SIMULTANEOUS,
                false,
                secrets,
            ),
            shared_attempts(),
        )
    }

    async fn seed_agent_prompt(
        handles: &TestStateHandles,
        agent_name: &str,
        prompt: &str,
    ) -> anyhow::Result<()> {
        use crate::domain::documents::Document;
        let path = format!("/agents/{agent_name}/prompt.md");
        let doc = Document {
            title: format!("{agent_name} prompt"),
            body_markdown: prompt.to_string(),
            path: Some(path.parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    fn repository() -> (RepoName, Repository) {
        let repo_name = RepoName::from_str("dourolabs/metis").expect("repo name should parse");
        let repository = Repository::new(
            "https://github.com/dourolabs/metis.git".to_string(),
            Some("main".to_string()),
            Some("repo-image".to_string()),
            None,
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
        session: Session,
    ) -> anyhow::Result<()> {
        let (task_id, _) = handles
            .store
            .add_session(session, Utc::now(), &ActorRef::test())
            .await?;
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
            .transition_task_to_completion(&task_id, Ok(()), None, ActorRef::test())
            .await?;
        Ok(())
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
            status,
            assignee.map(str::to_string),
            Some(session_settings(repo_name)),
            Vec::new(),
            dependencies,
            Vec::new(),
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
            status,
            assignee: assignee.map(str::to_string),
            session_settings: SessionSettings::default(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
        }
    }

    fn task(
        prompt: &str,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<&str>,
        env_vars: HashMap<String, String>,
    ) -> Session {
        Session::new(
            prompt.to_string(),
            context,
            spawned_from,
            Username::from("test-creator"),
            image.map(str::to_string),
            None,
            env_vars,
            None,
            None,
            None,
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
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2);

        let mut issue_ids = HashSet::new();
        let mut spawned_from_issue_ids = HashSet::new();
        let default_branch = "main".to_string();
        for task in tasks {
            let Session {
                prompt,
                context,
                spawned_from,
                env_vars,
                ..
            } = task;

            assert_eq!(prompt, "Fix the issue");
            assert_eq!(
                context,
                BundleSpec::ServiceRepository {
                    name: repo_name.clone(),
                    rev: Some(default_branch.clone())
                }
            );
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
                    BundleSpec::None,
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

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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
            None,
            Username::from("test-creator"),
            Vec::new(),
            repo_name.clone(),
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = handles.store.add_patch(patch, &ActorRef::test()).await?;
        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Review patch".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    session_settings: session_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: vec![patch_id.clone()],
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        let mut tasks = queue("agent-a").spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        let updated_patch = handles.store.get_patch(&patch_id, false).await?;
        let mut updated_patch = updated_patch.item;
        updated_patch.status = PatchStatus::ChangesRequested;
        updated_patch.diff = "diff --git a/file b/file\n+change\n".to_string();
        updated_patch.reviews = vec![Review::new(
            "needs adjustments".to_string(),
            false,
            "reviewer".to_string(),
            None,
        )];
        handles
            .store
            .update_patch(&patch_id, updated_patch, &ActorRef::test())
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn task_assignee_mismatch_skips_for_non_assignee() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
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

        let tasks = queue("agent-b").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn assignment_agent_spawns_for_unassigned_issue() -> anyhow::Result<()> {
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
                    DEFAULT_AGENT_MAX_TRIES,
                    DEFAULT_AGENT_MAX_SIMULTANEOUS,
                    true,
                    Vec::new(),
                ),
                shared_attempts(),
            )
        };
        let tasks = q.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].context, BundleSpec::None);
        assert_eq!(
            tasks[0]
                .env_vars
                .get(ISSUE_ID_ENV_VAR)
                .map(|value| value.as_str()),
            Some(issue_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn non_assignment_agent_skips_unassigned_issue() -> anyhow::Result<()> {
        let handles = test_state_handles();
        handles
            .store
            .add_issue(
                issue_without_repo("Needs assignment", IssueStatus::Open, None),
                &ActorRef::test(),
            )
            .await?;

        let queue = queue("agent-a");
        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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

        handles
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

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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
                    BundleSpec::None,
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

        handles
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
        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn spawns_when_repo_missing_and_allows_missing_image() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Missing repo".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Missing image".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2);

        let has_repo_task = tasks.iter().any(|t| {
            matches!(
                t.context,
                BundleSpec::ServiceRepository { ref name, .. } if name == &repo_name
            )
        });
        assert!(has_repo_task, "expected a task with ServiceRepository");

        let has_no_repo_task = tasks.iter().any(|t| matches!(t.context, BundleSpec::None));
        assert!(
            has_no_repo_task,
            "expected a task with BundleSpec::None for the repo-less issue"
        );

        Ok(())
    }

    #[tokio::test]
    async fn uses_session_settings_max_retries_override() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 3;

        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Override retries".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    session_settings: session_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        let (task_id, _) = handles
            .store
            .add_session(
                Session {
                    prompt: "Existing".to_string(),
                    context: BundleSpec::None,
                    spawned_from: Some(issue_id.clone()),
                    creator: Username::from("test-creator"),
                    image: None,
                    model: None,
                    env_vars: HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                    cpu_limit: None,
                    memory_limit: None,
                    secrets: None,
                    status: Status::Created,
                    last_message: None,
                    error: None,
                    deleted: false,
                    creation_time: None,
                    start_time: None,
                    end_time: None,
                },
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    session_settings: session_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
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
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    session_settings: session_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;

        handles
            .store
            .add_session(
                Session {
                    prompt: "Pending work".to_string(),
                    context: BundleSpec::None,
                    spawned_from: Some(first_issue_id.clone()),
                    creator: Username::from("test-creator"),
                    image: None,
                    model: None,
                    env_vars: HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), first_issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                    cpu_limit: None,
                    memory_limit: None,
                    secrets: None,
                    status: Status::Created,
                    last_message: None,
                    error: None,
                    deleted: false,
                    creation_time: None,
                    start_time: None,
                    end_time: None,
                },
                Utc::now(),
                &ActorRef::test(),
            )
            .await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].env_vars.get(ISSUE_ID_ENV_VAR).map(String::as_str),
            Some(second_issue_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn enforces_max_spawn_attempts_per_state() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 2;

        let (handles, repo_name) = state_with_repository().await?;
        handles
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

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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

        let first_run = queue.spawn(&handles.state).await?;
        assert_eq!(first_run.len(), 1);
        assert!(queue.spawn(&handles.state).await?.is_empty());

        let issue = handles.store.get_issue(&issue_id, false).await?;
        let mut issue = issue.item;
        issue.status = IssueStatus::InProgress;
        handles
            .store
            .update_issue(&issue_id, issue, &ActorRef::test())
            .await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

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
        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        // Second attempt should be blocked (max_tries=1 reached).
        assert!(queue.spawn(&handles.state).await?.is_empty());

        // Create a child issue — this counts as progress on the parent.
        // Assign to a different agent so it doesn't spawn here.
        handles
            .store
            .add_issue(
                issue(
                    "Child issue",
                    IssueStatus::Open,
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
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].env_vars.get(ISSUE_ID_ENV_VAR).map(String::as_str),
            Some(parent_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_child_updated() -> anyhow::Result<()> {
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

        // Create a child before the first spawn.
        let (child_id, _) = handles
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

        // First spawn attempt succeeds.
        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2); // parent + child both eligible
        for t in tasks.drain(..) {
            record_completed_task(&handles, t).await?;
        }

        // Further attempts should be blocked for both.
        assert!(queue.spawn(&handles.state).await?.is_empty());

        // Update the child issue — this counts as progress on the parent.
        let child = handles.store.get_issue(&child_id, false).await?;
        let mut child_item = child.item;
        child_item.status = IssueStatus::InProgress;
        handles
            .store
            .update_issue(&child_id, child_item, &ActorRef::test())
            .await?;

        // Parent's counter should have reset (child version changed).
        // Child's counter should also reset (its status changed).
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2);

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

        // Create a child before the first spawn.
        handles
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

        // First spawn consumes both parent and child.
        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2);
        for t in tasks.drain(..) {
            record_completed_task(&handles, t).await?;
        }

        // No changes to children — counter should NOT reset, so no tasks spawn.
        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[test]
    fn builds_from_agent() {
        use crate::domain::agents::Agent;

        let agent = Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt.md".to_string(),
            5,
            10,
            true,
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
    async fn does_not_spawn_for_rejected_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(
                issue(
                    "Rejected issue",
                    IssueStatus::Rejected,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_for_failed_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
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

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

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
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        let resolved = handles.state.resolve_task(&tasks[0]).await?;
        assert_eq!(
            tasks[0].context,
            BundleSpec::ServiceRepository {
                name: repo_name.clone(),
                rev: Some(default_branch.clone())
            }
        );
        assert_eq!(
            resolved.context.bundle,
            Bundle::GitRepository {
                url: "https://github.com/dourolabs/metis.git".to_string(),
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
        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Issue with secrets".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert_eq!(task.secrets, Some(secrets));

        Ok(())
    }

    #[tokio::test]
    async fn spawner_handles_none_secrets() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository.clone()).await?;
        seed_agent_prompt(&handles, "agent-a", "Fix the issue").await?;
        handles
            .store
            .add_issue(
                Issue {
                    issue_type: IssueType::Task,
                    title: String::new(),
                    description: "Issue without secrets".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await?;
        let queue = queue("agent-a");

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert!(task.secrets.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn unread_message_does_not_trigger_spawn_for_not_ready_issue() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        // Create a blocker so the issue is not ready by normal rules.
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
                    "Blocked but has messages",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id,
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Without unread messages, should not spawn.
        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty(), "should not spawn without unread messages");

        // Add an unread message for the issue.
        let recipient = ActorId::Issue(issue_id.clone());
        let msg = Message::new(None, recipient, "please look at this".to_string());
        handles.store.add_message(msg, &ActorRef::test()).await?;

        // Should still not spawn — unread messages no longer bypass readiness checks.
        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(
            tasks.is_empty(),
            "should not spawn for not-ready issue even with unread messages"
        );

        Ok(())
    }

    #[tokio::test]
    async fn read_messages_do_not_trigger_spawn() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        // Create a blocker so the issue is not ready by normal rules.
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
                    "Blocked with read messages",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id,
                    )],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Add a message that is already read.
        let recipient = ActorId::Issue(issue_id);
        let mut msg = Message::new(None, recipient, "already read".to_string());
        msg.is_read = true;
        handles.store.add_message(msg, &ActorRef::test()).await?;

        // Should not spawn because the only message is read.
        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn unread_messages_with_active_task_do_not_trigger_duplicate() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Issue with active task",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // Create an active task for this issue.
        let (task_id, _) = handles
            .store
            .add_session(
                task(
                    "Existing task",
                    BundleSpec::None,
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
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await?;

        // Add an unread message.
        let recipient = ActorId::Issue(issue_id);
        let msg = Message::new(None, recipient, "new message".to_string());
        handles.store.add_message(msg, &ActorRef::test()).await?;

        // Should not spawn a duplicate task even with unread messages.
        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn retry_counter_does_not_reset_on_new_unread_message() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.agent.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let (issue_id, _) = handles
            .store
            .add_issue(
                issue(
                    "Retry with messages",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ),
                &ActorRef::test(),
            )
            .await?;

        // First spawn attempt succeeds (attempt 1 of 1).
        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        // Second attempt should be blocked (max_tries=1 reached).
        assert!(queue.spawn(&handles.state).await?.is_empty());

        // Send an unread message — this should NOT reset the retry counter.
        let recipient = ActorId::Issue(issue_id.clone());
        let msg = Message::new(None, recipient, "new info".to_string());
        handles.store.add_message(msg, &ActorRef::test()).await?;

        // Spawning should still be blocked after max_tries even with new messages.
        assert!(queue.spawn(&handles.state).await?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn shared_spawn_attempts_persist_across_queue_instances() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
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
        let mut tasks = queue1.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        // Second iteration: new AgentQueue with same shared state.
        let mut queue2 = queue_with_attempts("agent-a", attempts.clone());
        queue2.agent.max_tries = 2;

        // Should still spawn (attempt 2 of 2).
        let mut tasks = queue2.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&handles, tasks.remove(0)).await?;

        // Third iteration: new AgentQueue, same shared state.
        let mut queue3 = queue_with_attempts("agent-a", attempts);
        queue3.agent.max_tries = 2;

        // Should be blocked (max_tries=2 reached across iterations).
        let tasks = queue3.spawn(&handles.state).await?;
        assert!(
            tasks.is_empty(),
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
        handles
            .store
            .add_issue(issue_with_secrets, &ActorRef::test())
            .await?;

        // Agent has its own secrets, one overlapping with the issue.
        let queue = queue_with_secrets(
            "agent-a",
            vec!["OPENAI_API_KEY".to_string(), "CUSTOM_KEY".to_string()],
        );
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        let session = &tasks[0];
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
        handles
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

        let queue = queue_with_secrets("agent-a", vec!["AGENT_SECRET".to_string()]);
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        assert_eq!(tasks[0].secrets, Some(vec!["AGENT_SECRET".to_string()]));

        Ok(())
    }

    #[tokio::test]
    async fn no_secrets_when_agent_and_issue_have_none() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;

        handles
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
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        assert_eq!(tasks[0].secrets, None);

        Ok(())
    }
}
