use crate::{app::AppState, background::WorkerOutcome};
use chrono::Utc;
use tracing::{info, warn};

/// Run configured spawners once to enqueue new work.
pub async fn run_spawners(state: &AppState) -> WorkerOutcome {
    if state.spawners.is_empty() {
        return WorkerOutcome::Idle;
    }

    let mut processed = 0usize;
    let mut failed = 0usize;

    for spawner in &state.spawners {
        match spawner.spawn(state).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    continue;
                }

                info!(
                    spawner = spawner.name(),
                    count = tasks.len(),
                    "spawner produced tasks"
                );

                let mut store = state.store.write().await;
                for task in tasks {
                    match store.add_task(task, Utc::now()).await {
                        Ok(metis_id) => {
                            processed += 1;
                            info!(
                                spawner = spawner.name(),
                                metis_id = %metis_id,
                                "added task produced by spawner"
                            );
                        }
                        Err(err) => {
                            failed += 1;
                            warn!(
                                spawner = spawner.name(),
                                error = %err,
                                "failed to add task from spawner"
                            );
                        }
                    }
                }
            }
            Err(err) => {
                failed += 1;
                warn!(spawner = spawner.name(), error = %err, "spawner run failed")
            }
        }
    }

    if processed == 0 && failed == 0 {
        WorkerOutcome::Idle
    } else {
        WorkerOutcome::Progress { processed, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ServiceState,
        background::Spawner,
        config::{
            AppConfig, BackgroundSection, GithubPollerConfig, KubernetesSection, MetisSection,
            SchedulerConfig, ServiceSection,
        },
        store::{MemoryStore, Task},
    };
    use async_trait::async_trait;
    use metis_common::jobs::BundleSpec;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::RwLock;

    fn base_state(spawners: Vec<Arc<dyn Spawner>>) -> AppState {
        AppState {
            config: Arc::new(AppConfig {
                metis: MetisSection::default(),
                kubernetes: KubernetesSection::default(),
                service: ServiceSection::default(),
                background: BackgroundSection {
                    github_poller: GithubPollerConfig::default(),
                    scheduler: SchedulerConfig::default(),
                    agent_queues: Vec::new(),
                },
            }),
            service_state: Arc::new(ServiceState::default()),
            store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
            job_engine: Arc::new(crate::job_engine::MockJobEngine::new()),
            spawners,
        }
    }

    #[tokio::test]
    async fn idle_when_no_spawners() {
        let state = base_state(Vec::new());
        assert_eq!(run_spawners(&state).await, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn records_spawned_tasks() {
        let spawner = Arc::new(StubSpawner {
            name: "stub",
            tasks: vec![Task {
                prompt: "hello".into(),
                context: BundleSpec::None,
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            }],
        });
        let state = base_state(vec![spawner]);

        let outcome = run_spawners(&state).await;
        assert!(matches!(outcome, WorkerOutcome::Progress { processed, .. } if processed == 1));

        let tasks = state.store.read().await.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
    }

    struct StubSpawner {
        name: &'static str,
        tasks: Vec<Task>,
    }

    #[async_trait]
    impl Spawner for StubSpawner {
        fn name(&self) -> &str {
            self.name
        }

        async fn spawn(&self, _state: &AppState) -> anyhow::Result<Vec<Task>> {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(self.tasks.clone())
        }
    }
}
