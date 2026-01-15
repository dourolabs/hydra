use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    store::Status,
};
use async_trait::async_trait;
use tracing::{error, info};

/// Scheduled worker that monitors running jobs once per iteration.
///
/// A successful iteration returns `Progress`, empty queues return `Idle`,
/// and store failures map to `TransientError` so the scheduler can back off.
#[derive(Clone)]
pub struct MonitorRunningJobsWorker {
    state: AppState,
}

impl MonitorRunningJobsWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ScheduledWorker for MonitorRunningJobsWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        // Kill any jobs that are running in the engine but missing from the store
        self.state.reap_orphaned_jobs().await;

        let running_ids = {
            let store = self.state.store.read().await;
            match store.list_tasks_with_status(Status::Running).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list running tasks");
                    return WorkerOutcome::TransientError {
                        reason: err.to_string(),
                    };
                }
            }
        };

        if running_ids.is_empty() {
            return WorkerOutcome::Idle;
        }

        info!(count = running_ids.len(), "found running tasks to monitor");

        // Check each running job's status
        for metis_id in &running_ids {
            self.state.reconcile_running_task(metis_id.clone()).await;
        }

        WorkerOutcome::Progress {
            processed: running_ids.len(),
            failed: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        job_engine::{JobStatus, MockJobEngine},
        store::{Status, Task},
        test::{store::FailingStore, test_state, test_state_with_engine},
    };
    use chrono::Utc;
    use metis_common::jobs::BundleSpec;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn returns_idle_when_no_running_tasks_exist() {
        let state = test_state();
        let worker = MonitorRunningJobsWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn reconciles_running_jobs_and_reports_progress() {
        let engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(engine.clone());
        let mut store = state.store.write().await;
        let task = Task {
            prompt: "observe".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: None,
            env_vars: HashMap::new(),
        };
        let task_id = store
            .add_task(task, Utc::now())
            .await
            .expect("task should be added");
        store
            .mark_task_running(&task_id, Utc::now())
            .await
            .expect("task should be marked running");
        drop(store);

        engine.insert_job(&task_id, JobStatus::Running).await;

        let worker = MonitorRunningJobsWorker::new(state.clone());
        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        );

        let store = state.store.read().await;
        assert_eq!(
            store
                .get_status(&task_id)
                .await
                .expect("status should exist"),
            Status::Running
        );
    }

    #[tokio::test]
    async fn returns_transient_error_when_store_fails() {
        let mut state = test_state();
        state.store = Arc::new(RwLock::new(Box::new(FailingStore)));
        let worker = MonitorRunningJobsWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }
}
