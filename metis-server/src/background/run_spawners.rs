use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
};
use async_trait::async_trait;
use chrono::Utc;
use tracing::{info, warn};

const WORKER_NAME: &str = "run_spawners";

/// Scheduled worker that runs configured spawners once per iteration.
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
        if self.state.spawners.is_empty() {
            info!(worker = WORKER_NAME, "no spawners configured; worker idle");
            return WorkerOutcome::Idle;
        }

        let mut processed = 0usize;
        let mut failure_reason: Option<String> = None;

        for spawner in &self.state.spawners {
            match spawner.spawn(&self.state).await {
                Ok(tasks) => {
                    if tasks.is_empty() {
                        continue;
                    }

                    info!(
                        worker = WORKER_NAME,
                        spawner = spawner.name(),
                        count = tasks.len(),
                        "spawner produced tasks"
                    );

                    let mut store = self.state.store.write().await;
                    for task in tasks {
                        match store.add_task(task, Utc::now()).await {
                            Ok(metis_id) => {
                                processed += 1;
                                info!(
                                    worker = WORKER_NAME,
                                    spawner = spawner.name(),
                                    metis_id = %metis_id,
                                    "added task produced by spawner"
                                );
                            }
                            Err(err) => {
                                if failure_reason.is_none() {
                                    failure_reason = Some(err.to_string());
                                }
                                warn!(
                                    spawner = spawner.name(),
                                    worker = WORKER_NAME,
                                    error = %err,
                                    "failed to add task from spawner"
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
                        spawner = spawner.name(),
                        error = %err,
                        "spawner run failed"
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
                "spawners produced no tasks; worker idle"
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
    use crate::domain::jobs::BundleSpec;
    use crate::{background::spawner::Spawner, test::test_state};
    use anyhow::anyhow;
    use std::{collections::HashMap, sync::Arc};

    #[derive(Clone)]
    enum SpawnOutcome {
        Tasks(Vec<crate::store::Task>),
        Error(String),
    }

    #[derive(Clone)]
    struct TestSpawner {
        name: &'static str,
        outcome: SpawnOutcome,
    }

    #[async_trait]
    impl Spawner for TestSpawner {
        fn name(&self) -> &str {
            self.name
        }

        async fn spawn(&self, _state: &AppState) -> anyhow::Result<Vec<crate::store::Task>> {
            match &self.outcome {
                SpawnOutcome::Tasks(tasks) => Ok(tasks.clone()),
                SpawnOutcome::Error(err) => Err(anyhow!(err.clone())),
            }
        }
    }

    fn make_task(prompt: &str) -> crate::store::Task {
        crate::store::Task::new(
            prompt.to_string(),
            BundleSpec::None,
            None,
            None,
            HashMap::new(),
            None,
        )
    }

    #[tokio::test]
    async fn returns_idle_when_no_spawners_configured() {
        let worker = RunSpawnersWorker::new(test_state());

        assert_eq!(worker.run_iteration().await, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn enqueues_tasks_and_reports_progress() {
        let mut state = test_state();
        state.spawners = vec![Arc::new(TestSpawner {
            name: "static",
            outcome: SpawnOutcome::Tasks(vec![make_task("spawn me")]),
        })];
        let worker = RunSpawnersWorker::new(state.clone());

        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        );

        let store = state.store.read().await;
        let tasks = store
            .list_tasks()
            .await
            .expect("tasks should be listed without error");
        assert_eq!(tasks.len(), 1);
    }

    #[tokio::test]
    async fn surfaces_errors_from_spawners() {
        let mut state = test_state();
        state.spawners = vec![Arc::new(TestSpawner {
            name: "failing",
            outcome: SpawnOutcome::Error("spawn failed".to_string()),
        })];
        let worker = RunSpawnersWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }
}
