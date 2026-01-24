#[cfg(test)]
use crate::domain::issues::{IssueDependency, IssueType};
#[cfg(test)]
use crate::domain::users::Username;
use crate::{
    app::AppState,
    config::AgentQueueConfig,
    domain::{
        issues::{Issue, IssueDependencyType, IssueStatus},
        jobs::BundleSpec,
    },
    store::{Status, Store, StoreError, Task},
};
use anyhow::Context;
use async_trait::async_trait;
use metis_common::IssueId;
#[cfg(test)]
use metis_common::RepoName;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::str::FromStr;
use tokio::sync::RwLock;

pub const ISSUE_ID_ENV_VAR: &str = "METIS_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "METIS_AGENT_NAME";

#[async_trait]
pub trait Spawner: Send + Sync {
    fn name(&self) -> &str;
    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>>;
}

#[derive(Clone, Copy, Debug)]
struct SpawnAttempt {
    status: IssueStatus,
    attempts: u32,
}

pub struct AgentQueue {
    pub name: String,
    pub prompt: String,
    pub max_tries: u32,
    pub max_simultaneous: u32,
    spawn_attempts: RwLock<HashMap<IssueId, SpawnAttempt>>,
}

impl AgentQueue {
    pub fn from_config(config: &AgentQueueConfig) -> Self {
        Self {
            name: config.name.clone(),
            prompt: config.prompt.clone(),
            max_tries: config.max_tries,
            max_simultaneous: config.max_simultaneous,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
    }

    async fn build_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> anyhow::Result<Option<Task>> {
        let repo_name = match issue.job_settings.repo_name.clone() {
            Some(repo_name) => repo_name,
            None => return Ok(None),
        };

        let image = match issue.job_settings.image.as_ref() {
            Some(image) if !image.trim().is_empty() => image.trim().to_string(),
            _ => return Ok(None),
        };

        let repository = state
            .repository_from_store(&repo_name)
            .await
            .context("failed to load repository for issue task")?;
        let rev = issue
            .job_settings
            .branch
            .clone()
            .or_else(|| repository.default_branch.clone());

        let mut env_vars = HashMap::new();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string());
        env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), self.name.clone());
        let mut job_settings = issue.job_settings.clone();
        job_settings.image = Some(image);

        Ok(Some(Task::new(
            self.prompt.clone(),
            BundleSpec::ServiceRepository {
                name: repo_name,
                rev,
            },
            Some(issue_id.clone()),
            None,
            env_vars,
            Some(job_settings),
        )))
    }

    async fn register_spawn_attempt(
        &self,
        issue_id: &IssueId,
        status: IssueStatus,
        max_tries: u32,
    ) -> bool {
        let mut attempts = self.spawn_attempts.write().await;
        let entry = attempts.entry(issue_id.clone()).or_insert(SpawnAttempt {
            status,
            attempts: 0,
        });

        if entry.status != status {
            *entry = SpawnAttempt {
                status,
                attempts: 0,
            };
        }

        if entry.attempts >= max_tries {
            return false;
        }

        entry.attempts += 1;
        true
    }

    fn max_tries_for_issue(&self, issue: &Issue) -> u32 {
        issue.job_settings.max_retries.unwrap_or(self.max_tries)
    }
}

#[async_trait]
impl Spawner for AgentQueue {
    fn name(&self) -> &str {
        &self.name
    }

    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>> {
        let store = state.store.read().await;

        let task_state = agent_task_state(store.as_ref(), &self.name)
            .await
            .context("failed to list tasks for agent queue")?;

        let max_simultaneous = self.max_simultaneous as usize;
        if max_simultaneous == 0 {
            return Ok(Vec::new());
        }

        let active_tasks = task_state.running_tasks + task_state.pending_tasks;
        if active_tasks >= max_simultaneous {
            return Ok(Vec::new());
        }

        let mut remaining_capacity = max_simultaneous - active_tasks;

        let issues = store
            .list_issues()
            .await
            .context("failed to list issues for agent queue")?;

        let mut tasks = Vec::new();
        for (issue_id, issue) in issues {
            if issue.assignee.as_deref() != Some(self.name.as_str()) {
                continue;
            }

            // Do not spawn tasks for closed or dropped issues.
            if matches!(issue.status, IssueStatus::Closed | IssueStatus::Dropped) {
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

            if parent_has_running_task(store.as_ref(), &issue).await? {
                continue;
            }

            let maybe_task = self.build_task(state, &issue_id, &issue).await?;
            let Some(task) = maybe_task else {
                continue;
            };

            let max_tries = self.max_tries_for_issue(&issue);
            if !self
                .register_spawn_attempt(&issue_id, issue.status, max_tries)
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

struct AgentTaskState {
    existing_issue_ids: HashSet<IssueId>,
    running_tasks: usize,
    pending_tasks: usize,
}

async fn agent_task_state(
    store: &dyn Store,
    agent_name: &str,
) -> Result<AgentTaskState, StoreError> {
    let mut state = AgentTaskState {
        existing_issue_ids: HashSet::new(),
        running_tasks: 0,
        pending_tasks: 0,
    };
    let task_ids = store.list_tasks().await?;

    for task_id in task_ids {
        if let Ok(Task { env_vars, .. }) = store.get_task(&task_id).await {
            if !matches!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(current) if current == agent_name
            ) {
                continue;
            }

            let status = store.get_status(&task_id).await?;
            match status {
                Status::Pending => state.pending_tasks += 1,
                Status::Running => state.running_tasks += 1,
                _ => {}
            }

            if let Some(issue_id) = env_vars
                .get(ISSUE_ID_ENV_VAR)
                .and_then(|value| value.parse::<IssueId>().ok())
            {
                if matches!(status, Status::Pending | Status::Running) {
                    state.existing_issue_ids.insert(issue_id);
                }
            }
        }
    }

    Ok(state)
}

async fn parent_has_running_task(store: &dyn Store, issue: &Issue) -> Result<bool, StoreError> {
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
    {
        for task_id in store.get_tasks_for_issue(&dependency.issue_id).await? {
            if matches!(store.get_status(&task_id).await?, Status::Running) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::JobSettings;
    use crate::domain::jobs::{Bundle, BundleSpec};
    use crate::{
        app::ServiceRepository,
        config::{AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
        test::test_state_with_repo,
    };
    use chrono::Utc;

    fn default_user() -> Username {
        Username::from("spawner")
    }

    fn queue(agent_name: &str) -> AgentQueue {
        AgentQueue {
            name: agent_name.to_string(),
            prompt: "Fix the issue".to_string(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
    }

    fn repository() -> (RepoName, ServiceRepository) {
        let repo_name = RepoName::from_str("dourolabs/metis").expect("repo name should parse");
        let repository = ServiceRepository::new(
            repo_name.clone(),
            "https://github.com/dourolabs/metis.git".to_string(),
            Some("main".to_string()),
            Some("repo-image".to_string()),
        );

        (repo_name, repository)
    }

    fn job_settings(repo_name: &RepoName) -> JobSettings {
        JobSettings {
            repo_name: Some(repo_name.clone()),
            image: Some("repo-image".to_string()),
            ..JobSettings::default()
        }
    }

    async fn state_with_repository() -> anyhow::Result<(AppState, RepoName)> {
        let (repo_name, repository) = repository();
        let state = test_state_with_repo(repository).await?;
        Ok((state, repo_name))
    }

    async fn record_completed_task(state: &AppState, task: Task) -> anyhow::Result<()> {
        let mut store = state.store.write().await;
        let task_id = store.add_task(task, Utc::now()).await?;
        store.mark_task_running(&task_id, Utc::now()).await?;
        store
            .mark_task_complete(&task_id, Ok(()), None, Utc::now())
            .await?;
        Ok(())
    }

    fn issue(
        description: &str,
        status: IssueStatus,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        repo_name: &RepoName,
    ) -> Issue {
        Issue::new(
            IssueType::Task,
            description.to_string(),
            default_user(),
            String::new(),
            status,
            assignee.map(str::to_string),
            Some(job_settings(repo_name)),
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    fn task(
        prompt: &str,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<&str>,
        env_vars: HashMap<String, String>,
    ) -> Task {
        Task::new(
            prompt.to_string(),
            context,
            spawned_from,
            image.map(str::to_string),
            env_vars,
            None,
        )
    }

    #[tokio::test]
    async fn spawns_tasks_for_ready_assigned_issues() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        let assigned_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Fix login page",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        let in_progress_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "In-progress but ready",
                    IssueStatus::InProgress,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Ignore closed",
                    IssueStatus::Closed,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?;
        }

        let queue = queue("agent-a");
        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 2);

        let mut issue_ids = HashSet::new();
        let mut spawned_from_issue_ids = HashSet::new();
        let default_branch = "main".to_string();
        for task in tasks {
            let Task {
                prompt,
                context,
                spawned_from,
                env_vars,
                ..
            } = task;

            assert_eq!(prompt, "Fix the issue".to_string());
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
        let (state, repo_name) = state_with_repository().await?;
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Already queued",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_task(
                    task(
                        "Fix the issue",
                        BundleSpec::None,
                        Some(issue_id.clone()),
                        Some("metis-worker:latest"),
                        HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                    ),
                    Utc::now(),
                )
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_issue_not_ready() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        let blocker_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Blocker",
                    IssueStatus::Open,
                    None,
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Blocked issue",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id.clone(),
                    )],
                    &repo_name,
                ))
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_parent_task_running() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        let parent_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Parent issue",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        {
            let mut store = state.store.write().await;
            let task_id = store
                .add_task(
                    task(
                        "Parent task",
                        BundleSpec::None,
                        Some(parent_id.clone()),
                        Some("metis-worker:latest"),
                        HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), parent_id.to_string()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                    ),
                    Utc::now(),
                )
                .await?;
            store.mark_task_running(&task_id, Utc::now()).await?;
        }

        {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Child issue",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![IssueDependency::new(
                        IssueDependencyType::ChildOf,
                        parent_id.clone(),
                    )],
                    &repo_name,
                ))
                .await?;
        }

        let queue = queue("agent-a");
        let tasks = queue.spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_repo_or_image_missing() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Missing repo".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: JobSettings {
                        repo_name: None,
                        remote_url: None,
                        image: Some("metis-worker:latest".to_string()),
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                    },
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;

            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Missing image".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: JobSettings {
                        repo_name: Some(repo_name),
                        remote_url: None,
                        image: None,
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                    },
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn uses_job_settings_max_retries_override() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 3;

        let (state, repo_name) = state_with_repository().await?;
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Override retries".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: JobSettings {
                        repo_name: Some(repo_name),
                        remote_url: None,
                        image: Some("metis-worker:latest".to_string()),
                        branch: Some("main".to_string()),
                        max_retries: Some(1),
                        cpu_limit: None,
                        memory_limit: None,
                    },
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;
        }

        let mut tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&state, tasks.remove(0)).await?;

        let tasks = queue.spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_at_max_simultaneous() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_simultaneous = 1;

        let (state, repo_name) = state_with_repository().await?;
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Already running".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: job_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            let task_id = store
                .add_task(
                    Task {
                        prompt: "Existing".to_string(),
                        context: BundleSpec::None,
                        spawned_from: Some(issue_id.clone()),
                        image: None,
                        env_vars: HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                        job_settings: JobSettings::default(),
                    },
                    Utc::now(),
                )
                .await?;
            store.mark_task_running(&task_id, Utc::now()).await?;
        }

        let tasks = queue.spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn caps_new_tasks_to_remaining_capacity() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_simultaneous = 2;

        let (state, repo_name) = state_with_repository().await?;
        let first_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "First issue".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: job_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };
        let second_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Second issue".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: job_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_task(
                    Task {
                        prompt: "Pending work".to_string(),
                        context: BundleSpec::None,
                        spawned_from: Some(first_issue_id.clone()),
                        image: None,
                        env_vars: HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), first_issue_id.to_string()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                        job_settings: JobSettings::default(),
                    },
                    Utc::now(),
                )
                .await?;
        }

        let tasks = queue.spawn(&state).await?;
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
        queue.max_tries = 2;

        let (state, repo_name) = state_with_repository().await?;
        {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "Retry limited",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?;
        }

        let mut tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&state, tasks.remove(0)).await?;

        let mut tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(&state, tasks.remove(0)).await?;

        let tasks = queue.spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_status_changes() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 1;

        let (state, repo_name) = state_with_repository().await?;
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue(
                    "State change reset",
                    IssueStatus::Open,
                    Some("agent-a"),
                    vec![],
                    &repo_name,
                ))
                .await?
        };

        let first_run = queue.spawn(&state).await?;
        assert_eq!(first_run.len(), 1);
        assert!(queue.spawn(&state).await?.is_empty());

        {
            let mut store = state.store.write().await;
            let mut issue = store.get_issue(&issue_id).await?;
            issue.status = IssueStatus::InProgress;
            store.update_issue(&issue_id, issue).await?;
        }

        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }

    #[test]
    fn builds_from_config() {
        let config = AgentQueueConfig {
            name: "agent-config".to_string(),
            prompt: "Handle issues".to_string(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
        };

        let queue = AgentQueue::from_config(&config);

        assert_eq!(queue.name, "agent-config");
        assert_eq!(queue.prompt, "Handle issues");
        assert_eq!(queue.max_tries, DEFAULT_AGENT_MAX_TRIES);
        assert_eq!(queue.max_simultaneous, DEFAULT_AGENT_MAX_SIMULTANEOUS);
    }

    #[tokio::test]
    async fn service_repo_context_uses_repo_defaults() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let default_branch = repository
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".into());
        let default_image = "agent-image".to_string();
        let state = test_state_with_repo(repository).await?;
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Assigned".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: JobSettings {
                        repo_name: Some(repo_name.clone()),
                        remote_url: None,
                        image: Some(default_image.clone()),
                        branch: None,
                        max_retries: None,
                        cpu_limit: None,
                        memory_limit: None,
                    },
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };
        let queue = queue("agent-a");

        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        let resolved = state.resolve_task(&tasks[0]).await?;
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
        assert_eq!(resolved.env_vars.get("METIS_GITHUB_TOKEN"), None);
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
    async fn sets_creator_github_token_env_var() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Needs token".to_string(),
                    creator: default_user(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: job_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;
        }

        let queue = queue("agent-a");
        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn skips_empty_creator_github_token_env_var() -> anyhow::Result<()> {
        let (state, repo_name) = state_with_repository().await?;
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Empty token".to_string(),
                    creator: Username::from("spawner"),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    job_settings: job_settings(&repo_name),
                    todo_list: Vec::new(),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;
        }

        let queue = queue("agent-a");
        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }
}
