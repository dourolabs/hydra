use crate::{
    AppState,
    config::AgentQueueConfig,
    store::{Status, Store, StoreError, Task},
};
use anyhow::Context;
use async_trait::async_trait;
#[cfg(test)]
use metis_common::issues::{IssueDependency, IssueDependencyType, IssueType};
use metis_common::{
    constants::ENV_GH_TOKEN,
    issues::IssueId,
    issues::{Issue, IssueStatus},
    jobs::BundleSpec,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

pub const ISSUE_ID_ENV_VAR: &str = "METIS_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "METIS_AGENT_NAME";

#[async_trait]
pub trait Spawner: Send + Sync {
    fn name(&self) -> &str;
    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>>;
}

pub const DEFAULT_AGENT_PROGRAM: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../metis/scripts/default_codex_prompt.rhai"
));

#[derive(Clone, Copy, Debug)]
struct SpawnAttempt {
    status: IssueStatus,
    attempts: u32,
}

pub struct AgentQueue {
    pub name: String,
    pub prompt: String,
    pub context_spec: BundleSpec,
    pub image: Option<String>,
    pub fallback_image: String,
    pub env_vars: HashMap<String, String>,
    pub max_tries: u32,
    spawn_attempts: RwLock<HashMap<IssueId, SpawnAttempt>>,
}

impl AgentQueue {
    pub fn from_config(config: &AgentQueueConfig, default_image: &str) -> Self {
        Self {
            name: config.name.clone(),
            prompt: config.prompt.clone(),
            context_spec: config.context.clone(),
            image: config.image.clone(),
            fallback_image: default_image.to_string(),
            env_vars: config.env_vars.clone(),
            max_tries: config.max_tries,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
    }

    fn build_task(&self, state: &AppState, issue_id: &IssueId) -> anyhow::Result<Task> {
        let mut env_vars = self.env_vars.clone();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string());
        env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), self.name.clone());

        let resolved = state
            .service_state
            .resolve_bundle_spec(self.context_spec.clone())
            .map_err(|err| anyhow::anyhow!("failed to resolve queue context: {err:?}"))?;
        if let Some(token) = resolved.github_token {
            env_vars.entry(ENV_GH_TOKEN.to_string()).or_insert(token);
        }

        let image = resolve_image(
            self.image.clone(),
            resolved.default_image,
            &self.fallback_image,
        )
        .context("failed to resolve queue image")?;

        Ok(Task {
            program: DEFAULT_AGENT_PROGRAM.to_string(),
            params: vec![self.prompt.clone()],
            context: resolved.bundle,
            image,
            env_vars,
        })
    }

    async fn register_spawn_attempt(&self, issue_id: &IssueId, status: IssueStatus) -> bool {
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

        if entry.attempts >= self.max_tries {
            return false;
        }

        entry.attempts += 1;
        true
    }
}

#[async_trait]
impl Spawner for AgentQueue {
    fn name(&self) -> &str {
        &self.name
    }

    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>> {
        let store = state.store.read().await;

        let existing_issue_ids = existing_issue_tasks_for_agent(store.as_ref(), &self.name)
            .await
            .context("failed to list tasks for agent queue")?;

        let issues = store
            .list_issues()
            .await
            .context("failed to list issues for agent queue")?;

        let mut tasks = Vec::new();
        for (issue_id, issue) in issues {
            let Issue {
                assignee, status, ..
            } = issue;
            if assignee.as_deref() != Some(self.name.as_str()) {
                continue;
            }

            // Do not spawn tasks for closed issues.
            if status == IssueStatus::Closed {
                continue;
            }

            let is_ready = store
                .is_issue_ready(&issue_id)
                .await
                .context("failed to determine if issue is ready")?;
            if !is_ready {
                continue;
            }

            if existing_issue_ids.contains(&issue_id) {
                continue;
            }

            if !self.register_spawn_attempt(&issue_id, status).await {
                continue;
            }

            let task = self.build_task(state, &issue_id)?;
            tasks.push(task);
        }

        Ok(tasks)
    }
}
fn resolve_image(
    user_supplied: Option<String>,
    repo_default: Option<String>,
    fallback: &str,
) -> Result<String, anyhow::Error> {
    if let Some(image) = user_supplied {
        let trimmed = image.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Some(default_image) = repo_default {
        let trimmed = default_image.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let trimmed = fallback.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "default worker image must not be empty for agent queue"
        ));
    }

    Ok(trimmed.to_string())
}

async fn existing_issue_tasks_for_agent(
    store: &dyn Store,
    agent_name: &str,
) -> Result<HashSet<IssueId>, StoreError> {
    let mut issue_ids = HashSet::new();
    let task_ids = store.list_tasks().await?;

    for task_id in task_ids {
        if let Ok(Task { env_vars, .. }) = store.get_task(&task_id).await {
            if !matches!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(current) if current == agent_name
            ) {
                continue;
            }

            if let Some(issue_id) = env_vars
                .get(ISSUE_ID_ENV_VAR)
                .and_then(|value| value.parse::<IssueId>().ok())
            {
                // Only consider tasks that are still actionable (not completed or failed).
                if matches!(
                    store.get_status(&task_id).await?,
                    Status::Pending | Status::Running
                ) {
                    issue_ids.insert(issue_id);
                }
            }
        }
    }

    Ok(issue_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{AgentQueueConfig, DEFAULT_AGENT_MAX_TRIES},
        state::{GitRepository, ServiceState},
        test::test_state,
    };
    use chrono::Utc;
    use metis_common::jobs::{Bundle, BundleSpec};
    use std::sync::Arc;

    fn queue(agent_name: &str) -> AgentQueue {
        AgentQueue {
            name: agent_name.to_string(),
            prompt: "Fix the issue".to_string(),
            context_spec: BundleSpec::None,
            image: None,
            fallback_image: "metis-worker:latest".to_string(),
            env_vars: HashMap::new(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
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

    #[tokio::test]
    async fn spawns_tasks_for_ready_assigned_issues() -> anyhow::Result<()> {
        let state = test_state();
        let assigned_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Fix login page".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };

        let in_progress_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "In-progress but ready".to_string(),
                    progress: String::new(),
                    status: IssueStatus::InProgress,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Ignore closed".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Closed,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert_eq!(tasks.len(), 2);

        let mut issue_ids = HashSet::new();
        for task in tasks {
            let Task {
                program,
                params,
                context,
                image,
                env_vars,
            } = task;

            assert_eq!(program, DEFAULT_AGENT_PROGRAM);
            assert_eq!(params, &["Fix the issue".to_string()]);
            assert_eq!(context, Bundle::None);
            assert_eq!(image, "metis-worker:latest");
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

        Ok(())
    }

    #[tokio::test]
    async fn does_not_requeue_when_task_exists() -> anyhow::Result<()> {
        let state = test_state();
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Already queued".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
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
                        program: DEFAULT_AGENT_PROGRAM.to_string(),
                        params: vec!["Fix the issue".to_string()],
                        context: Bundle::None,
                        image: "metis-worker:latest".to_string(),
                        env_vars: HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                    },
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
        let state = test_state();
        let blocker_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Blocker".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: None,
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Blocked issue".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: blocker_id.clone(),
                    }],
                    patches: Vec::new(),
                })
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn enforces_max_spawn_attempts_per_state() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 2;

        let state = test_state();
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Retry limited".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
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

        let state = test_state();
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "State change reset".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
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
    fn builds_from_config_with_default_image() {
        let config = AgentQueueConfig {
            name: "agent-config".to_string(),
            prompt: "Handle issues".to_string(),
            context: BundleSpec::None,
            image: None,
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            env_vars: HashMap::from([("CUSTOM".to_string(), "1".to_string())]),
        };

        let queue = AgentQueue::from_config(&config, "default-image");

        assert_eq!(queue.name, "agent-config");
        assert_eq!(queue.prompt, "Handle issues");
        assert_eq!(queue.image, None);
        assert_eq!(queue.fallback_image, "default-image");
        assert_eq!(queue.env_vars.get("CUSTOM"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn service_repo_context_uses_repo_defaults() -> anyhow::Result<()> {
        let mut state = test_state();
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([(
                "dourolabs/metis".to_string(),
                GitRepository {
                    name: "dourolabs/metis".to_string(),
                    remote_url: "https://github.com/dourolabs/metis.git".to_string(),
                    default_branch: Some("main".to_string()),
                    github_token: Some("token".to_string()),
                    default_image: Some("repo-image".to_string()),
                },
            )]),
        });
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "Assigned".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                    patches: Vec::new(),
                })
                .await?
        };
        let queue = AgentQueue {
            name: "agent-a".to_string(),
            prompt: "Do the thing".to_string(),
            context_spec: BundleSpec::ServiceRepository {
                name: "dourolabs/metis".to_string(),
                rev: None,
            },
            image: None,
            fallback_image: "default-image".to_string(),
            env_vars: HashMap::new(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            spawn_attempts: RwLock::new(HashMap::new()),
        };

        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        let Task {
            context,
            image,
            env_vars,
            ..
        } = &tasks[0];
        assert_eq!(
            context,
            &Bundle::GitRepository {
                url: "https://github.com/dourolabs/metis.git".to_string(),
                rev: "main".to_string(),
            }
        );
        assert_eq!(image, "repo-image");
        assert_eq!(env_vars.get(ENV_GH_TOKEN), Some(&"token".to_string()));
        assert_eq!(
            env_vars.get(ISSUE_ID_ENV_VAR).map(|value| value.as_str()),
            Some(issue_id.as_ref())
        );

        Ok(())
    }
}
