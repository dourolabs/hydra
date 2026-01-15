use crate::{
    app::AppState,
    background::WorkerOutcome,
    store::Status,
};
use tracing::{error, info};

/// Monitor running jobs once, reconciling orphaned engine jobs and task completion.
pub async fn monitor_running_jobs(state: &AppState) -> WorkerOutcome {
    let mut processed = 0usize;
    let mut failed = 0usize;

    let job_engine_jobs = match state.job_engine.list_jobs().await {
        Ok(jobs) => jobs,
        Err(err) => {
            error!(error = %err, "failed to list jobs in job engine");
            return WorkerOutcome::TransientError {
                reason: "list_jobs_failed".to_string(),
            };
        }
    };

    if !job_engine_jobs.is_empty() {
        state.reap_orphaned_jobs().await;
    }

    // Get running tasks
    let running_ids = {
        let store = state.store.read().await;
        match store.list_tasks_with_status(Status::Running).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list running tasks");
                return WorkerOutcome::TransientError {
                    reason: "list_running_failed".to_string(),
                };
            }
        }
    };

    if running_ids.is_empty() {
        return if job_engine_jobs.is_empty() {
            WorkerOutcome::Idle
        } else {
            WorkerOutcome::Progress { processed, failed }
        };
    }

    info!(count = running_ids.len(), "found running tasks to monitor");

    // Check each running job's status
    for metis_id in running_ids {
        state.reconcile_running_task(metis_id.clone()).await;
        processed += 1;

        match state.store.read().await.get_status(&metis_id).await {
            Ok(Status::Failed) => failed += 1,
            Ok(_) => {}
            Err(err) => {
                failed += 1;
                error!(
                    metis_id = %metis_id,
                    error = %err,
                    "failed to read status after reconciliation"
                );
            }
        };
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
        job_engine::{JobEngine, JobStatus, MockJobEngine},
        store::MemoryStore,
    };
    use std::sync::Arc;
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
    async fn idle_when_no_running_jobs() {
        let state = test_state(Arc::new(MockJobEngine::new()));
        assert_eq!(monitor_running_jobs(&state).await, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn kills_orphaned_engine_jobs() {
        let engine = Arc::new(MockJobEngine::new());
        let state = test_state(engine.clone());
        let task_id = metis_common::TaskId::new();
        engine.insert_job(&task_id, JobStatus::Running).await;

        let outcome = monitor_running_jobs(&state).await;
        assert!(!matches!(outcome, WorkerOutcome::Idle));

        let killed = engine.find_job_by_metis_id(&task_id).await.unwrap();
        assert_eq!(killed.status, JobStatus::Failed);
    }
}
