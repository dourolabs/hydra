use crate::{
    app::AppState,
    background::{
        Spawner,
        scheduler::{ScheduledWorker, WorkerOutcome},
    },
};
use async_trait::async_trait;
use chrono::Utc;
use tracing::{info, warn};

const WORKER_NAME: &str = "run_spawners";

/// Scheduled worker that runs configured agents once per iteration.
#[derive(Clone)]
pub struct RunSpawnersWorker {
    state: AppState,
}

impl RunSpawnersWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ScheduledWorker for RunSpawnersWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        info!(worker = WORKER_NAME, "worker iteration started");
        let agents = self.state.agent_queues().await;
        if agents.is_empty() {
            info!(worker = WORKER_NAME, "no agents configured; worker idle");
            return WorkerOutcome::Idle;
        }

        let mut processed = 0usize;
        let mut failure_reason: Option<String> = None;

        for agent in agents {
            match agent.spawn(&self.state).await {
                Ok(tasks) => {
                    if tasks.is_empty() {
                        continue;
                    }

                    info!(
                        worker = WORKER_NAME,
                        agent = agent.name(),
                        count = tasks.len(),
                        "agent produced tasks"
                    );

                    for task in tasks {
                        match self.state.add_task(task, Utc::now(), None).await {
                            Ok(metis_id) => {
                                processed += 1;
                                info!(
                                    worker = WORKER_NAME,
                                    agent = agent.name(),
                                    metis_id = %metis_id,
                                    "added task produced by agent"
                                );
                            }
                            Err(err) => {
                                if failure_reason.is_none() {
                                    failure_reason = Some(err.to_string());
                                }
                                warn!(
                                    agent = agent.name(),
                                    worker = WORKER_NAME,
                                    error = %err,
                                    "failed to add task from agent"
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    if failure_reason.is_none() {
                        failure_reason = Some(err.to_string());
                    }
                    warn!(
                        worker = WORKER_NAME,
                        agent = agent.name(),
                        error = %err,
                        "agent run failed"
                    );
                }
            }
        }

        if let Some(reason) = failure_reason {
            info!(
                worker = WORKER_NAME,
                "worker iteration completed with transient error"
            );
            return WorkerOutcome::TransientError { reason };
        }

        if processed == 0 {
            info!(
                worker = WORKER_NAME,
                "agents produced no tasks; worker idle"
            );
            WorkerOutcome::Idle
        } else {
            info!(
                worker = WORKER_NAME,
                processed, "worker iteration completed successfully"
            );
            WorkerOutcome::Progress {
                processed,
                failed: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::Repository,
        background::AgentQueue,
        config::{AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
        domain::issues::{Issue, IssueStatus, IssueType, JobSettings},
        domain::users::Username,
        test::{add_repository, test_state_handles},
    };
    use metis_common::RepoName;
    use std::{str::FromStr, sync::Arc};

    fn agent_queue_config(name: &str) -> AgentQueueConfig {
        AgentQueueConfig {
            name: name.to_string(),
            prompt: format!("prompt for {name}"),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
        }
    }

    fn issue_for_agent(agent: &str, repo_name: &RepoName) -> Issue {
        Issue::new(
            IssueType::Task,
            "Run agent".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open,
            Some(agent.to_string()),
            Some(JobSettings {
                repo_name: Some(repo_name.clone()),
                image: Some("agent-image".to_string()),
                ..JobSettings::default()
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn repository(_repo_name: &RepoName) -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("agent-image".to_string()),
        )
    }

    #[tokio::test]
    async fn returns_idle_when_no_agents_configured() {
        let handles = test_state_handles();
        let worker = RunSpawnersWorker::new(handles.state);

        assert_eq!(worker.run_iteration().await, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn enqueues_tasks_and_reports_progress() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let agent_name = "static";
        let repo_name = RepoName::from_str("dourolabs/metis")?;

        {
            let mut agents = handles.agents.write().await;
            *agents = vec![Arc::new(AgentQueue::from_config(&agent_queue_config(
                agent_name,
            )))];
        }

        add_repository(&handles.state, repo_name.clone(), repository(&repo_name)).await?;
        handles
            .store
            .add_issue(issue_for_agent(agent_name, &repo_name))
            .await?;

        let worker = RunSpawnersWorker::new(handles.state.clone());

        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        );

        let tasks = handles.state.list_tasks().await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn surfaces_errors_from_agents() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let agent_name = "failing";
        let repo_name = RepoName::from_str("missing/repo")?;

        {
            let mut agents = handles.agents.write().await;
            *agents = vec![Arc::new(AgentQueue::from_config(&agent_queue_config(
                agent_name,
            )))];
        }
        handles
            .store
            .add_issue(issue_for_agent(agent_name, &repo_name))
            .await?;
        let worker = RunSpawnersWorker::new(handles.state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));

        Ok(())
    }
}
