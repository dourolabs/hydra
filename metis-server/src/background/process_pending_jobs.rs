use crate::{app::AppState, background::WorkerOutcome, store::Status};
use tracing::{error, info, warn};

/// Process pending jobs once, spawning them in the job engine.
pub async fn process_pending_jobs(state: &AppState) -> WorkerOutcome {
    // Get pending tasks
    let pending_ids = {
        let store = state.store.read().await;
        match store.list_tasks_with_status(Status::Pending).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list pending tasks");
                return WorkerOutcome::TransientError {
                    reason: "list_pending_failed".to_string(),
                };
            }
        }
    };

    if pending_ids.is_empty() {
        return WorkerOutcome::Idle;
    }

    info!(count = pending_ids.len(), "found pending tasks to process");

    let mut processed = 0usize;
    let mut failed = 0usize;

    // Process each pending task
    for metis_id in pending_ids {
        state.start_pending_task(metis_id.clone()).await;
        processed += 1;

        match state.store.read().await.get_status(&metis_id).await {
            Ok(Status::Failed) => failed += 1,
            Ok(_) => {}
            Err(err) => {
                failed += 1;
                warn!(
                    metis_id = %metis_id,
                    error = %err,
                    "failed to read task status after spawn"
                );
            }
        }
    }

    WorkerOutcome::Progress { processed, failed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ServiceState,
        config::{
            AppConfig, BackgroundSection, GithubPollerConfig, KubernetesSection, MetisSection,
            SchedulerConfig, ServiceSection,
        },
        job_engine::{JobEngine, MockJobEngine},
        store::{MemoryStore, Task},
    };
    use metis_common::jobs::BundleSpec;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;

    fn test_state(engine: Arc<dyn JobEngine>) -> AppState {
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
            job_engine: engine,
            spawners: Vec::new(),
        }
    }

    #[tokio::test]
    async fn idle_when_no_pending_tasks() {
        let state = test_state(Arc::new(MockJobEngine::new()));
        let outcome = process_pending_jobs(&state).await;
        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn marks_pending_tasks_running() {
        let engine = Arc::new(MockJobEngine::new());
        let state = test_state(engine.clone());

        let task = Task {
            prompt: "do it".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: None,
            env_vars: HashMap::new(),
        };

        let task_id = {
            let mut store = state.store.write().await;
            store.add_task(task, chrono::Utc::now()).await.unwrap()
        };

        let outcome = process_pending_jobs(&state).await;
        assert!(matches!(outcome, WorkerOutcome::Progress { processed, .. } if processed == 1));

        let status = state.store.read().await.get_status(&task_id).await.unwrap();
        assert_eq!(status, crate::store::Status::Running);

        assert!(
            engine.env_vars_for_job(&task_id).is_some(),
            "job should be spawned"
        );
    }
}
