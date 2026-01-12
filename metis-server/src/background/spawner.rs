use crate::{
    AppState,
    config::AgentQueueConfig,
    store::{Status, Store, StoreError, Task},
};
use anyhow::Context;
use async_trait::async_trait;
use metis_common::{
    MetisId,
    artifacts::{Artifact, IssueStatus},
    constants::ENV_GH_TOKEN,
    jobs::BundleSpec,
};
use std::collections::{HashMap, HashSet};

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

pub struct AgentQueue {
    pub name: String,
    pub prompt: String,
    pub context_spec: BundleSpec,
    pub image: Option<String>,
    pub fallback_image: String,
    pub env_vars: HashMap<String, String>,
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
        }
    }

    fn build_task(&self, state: &AppState, issue_id: &MetisId) -> anyhow::Result<Task> {
        let mut env_vars = self.env_vars.clone();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.clone());
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

        Ok(Task::Spawn {
            program: DEFAULT_AGENT_PROGRAM.to_string(),
            params: vec![self.prompt.clone()],
            context: resolved.bundle,
            image,
            env_vars,
        })
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

        let artifacts = store
            .list_artifacts()
            .await
            .context("failed to list artifacts for agent queue")?;

        let mut tasks = Vec::new();
        for (artifact_id, artifact) in artifacts {
            if let Artifact::Issue {
                assignee, status, ..
            } = artifact
            {
                if assignee.as_deref() != Some(self.name.as_str()) {
                    continue;
                }

                // Only spawn tasks for open issues.
                if status != IssueStatus::Open {
                    continue;
                }

                if existing_issue_ids.contains(&artifact_id) {
                    continue;
                }

                let task = self.build_task(state, &artifact_id)?;
                tasks.push(task);
            }
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
) -> Result<HashSet<MetisId>, StoreError> {
    let mut issue_ids = HashSet::new();
    let task_ids = store.list_tasks().await?;

    for task_id in task_ids {
        if let Ok(Task::Spawn { env_vars, .. }) = store.get_task(&task_id).await {
            if !matches!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(current) if current == agent_name
            ) {
                continue;
            }

            match env_vars.get(ISSUE_ID_ENV_VAR) {
                Some(issue_id) => {
                    // Only consider tasks that are still actionable (not completed or failed).
                    if matches!(
                        store.get_status(&task_id).await?,
                        Status::Pending | Status::Running | Status::Blocked
                    ) {
                        issue_ids.insert(issue_id.clone());
                    }
                }
                None => continue,
            }
        }
    }

    Ok(issue_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AgentQueueConfig,
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
        }
    }

    #[tokio::test]
    async fn spawns_tasks_for_assigned_open_issues() -> anyhow::Result<()> {
        let state = test_state();
        let assigned_issue_id = {
            let mut store = state.store.write().await;
            store
                .add_artifact(Artifact::Issue {
                    issue_type: metis_common::artifacts::IssueType::Task,
                    description: "Fix login page".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_artifact(Artifact::Issue {
                    issue_type: metis_common::artifacts::IssueType::Task,
                    description: "Ignore closed".to_string(),
                    status: IssueStatus::Closed,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                })
                .await?;
        }

        {
            let mut store = state.store.write().await;
            store
                .add_artifact(Artifact::Issue {
                    issue_type: metis_common::artifacts::IssueType::Task,
                    description: "Ignore in-progress".to_string(),
                    status: IssueStatus::InProgress,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                })
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        match &tasks[0] {
            Task::Spawn {
                program,
                params,
                context,
                image,
                env_vars,
            } => {
                assert_eq!(program, DEFAULT_AGENT_PROGRAM);
                assert_eq!(params, &["Fix the issue".to_string()]);
                assert_eq!(context, &Bundle::None);
                assert_eq!(image, "metis-worker:latest");
                assert_eq!(env_vars.get(ISSUE_ID_ENV_VAR), Some(&assigned_issue_id));
                assert_eq!(
                    env_vars.get(AGENT_NAME_ENV_VAR),
                    Some(&"agent-a".to_string())
                );
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn does_not_requeue_when_task_exists() -> anyhow::Result<()> {
        let state = test_state();
        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_artifact(Artifact::Issue {
                    issue_type: metis_common::artifacts::IssueType::Task,
                    description: "Already queued".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
                })
                .await?
        };

        {
            let mut store = state.store.write().await;
            store
                .add_task(
                    Task::Spawn {
                        program: DEFAULT_AGENT_PROGRAM.to_string(),
                        params: vec!["Fix the issue".to_string()],
                        context: Bundle::None,
                        image: "metis-worker:latest".to_string(),
                        env_vars: HashMap::from([
                            (ISSUE_ID_ENV_VAR.to_string(), issue_id.clone()),
                            (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                        ]),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }

        let tasks = queue("agent-a").spawn(&state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[test]
    fn builds_from_config_with_default_image() {
        let config = AgentQueueConfig {
            name: "agent-config".to_string(),
            prompt: "Handle issues".to_string(),
            context: BundleSpec::None,
            image: None,
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
                .add_artifact(Artifact::Issue {
                    issue_type: metis_common::artifacts::IssueType::Task,
                    description: "Assigned".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("agent-a".to_string()),
                    dependencies: vec![],
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
        };

        let tasks = queue.spawn(&state).await?;
        assert_eq!(tasks.len(), 1);

        match &tasks[0] {
            Task::Spawn {
                context,
                image,
                env_vars,
                ..
            } => {
                assert_eq!(
                    context,
                    &Bundle::GitRepository {
                        url: "https://github.com/dourolabs/metis.git".to_string(),
                        rev: "main".to_string(),
                    }
                );
                assert_eq!(image, "repo-image");
                assert_eq!(env_vars.get(ENV_GH_TOKEN), Some(&"token".to_string()));
                assert_eq!(env_vars.get(ISSUE_ID_ENV_VAR), Some(&issue_id));
            }
        }

        Ok(())
    }
}
