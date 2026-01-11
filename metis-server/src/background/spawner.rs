use crate::{
    AppState,
    store::{Status, Store, StoreError, Task},
};
use anyhow::Context;
use async_trait::async_trait;
use metis_common::{
    MetisId,
    artifacts::{Artifact, IssueStatus},
    jobs::Bundle,
};
use std::collections::{HashMap, HashSet};

pub const ISSUE_ID_ENV_VAR: &str = "METIS_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "METIS_AGENT_NAME";

#[async_trait]
pub trait Spawner: Send + Sync {
    fn name(&self) -> &str;
    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>>;
}

pub struct AgentQueue {
    pub name: String,
    pub program: String,
    pub params: Vec<String>,
    pub context: Bundle,
    pub image: String,
    pub env_vars: HashMap<String, String>,
}

impl AgentQueue {
    fn build_task(&self, issue_id: &MetisId) -> Task {
        let mut env_vars = self.env_vars.clone();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.clone());
        env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), self.name.clone());

        Task::Spawn {
            program: self.program.clone(),
            params: self.params.clone(),
            context: self.context.clone(),
            image: self.image.clone(),
            env_vars,
        }
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

                // Skip issues that are already closed.
                if status == IssueStatus::Closed {
                    continue;
                }

                if existing_issue_ids.contains(&artifact_id) {
                    continue;
                }

                tasks.push(self.build_task(&artifact_id));
            }
        }

        Ok(tasks)
    }
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
    use crate::test::test_state;
    use chrono::Utc;

    fn queue(agent_name: &str) -> AgentQueue {
        AgentQueue {
            name: agent_name.to_string(),
            program: "0".to_string(),
            params: vec![],
            context: Bundle::None,
            image: "metis-worker:latest".to_string(),
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
                assert_eq!(program, "0");
                assert!(params.is_empty());
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
                        program: "0".to_string(),
                        params: vec![],
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
}
