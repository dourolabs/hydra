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
const WORKER_NAME: &str = "monitor_running_jobs";

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
        info!(worker = WORKER_NAME, "worker iteration started");
        // Kill any jobs that are running in the engine but missing from the store
        self.state.reap_orphaned_jobs().await;

        let running_ids = match self.state.list_tasks_with_status(Status::Running).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list running tasks");
                info!(
                    worker = WORKER_NAME,
                    "worker iteration completed with transient error"
                );
                return WorkerOutcome::TransientError {
                    reason: err.to_string(),
                };
            }
        };

        if running_ids.is_empty() {
            info!(worker = WORKER_NAME, "no running tasks found; worker idle");
            return WorkerOutcome::Idle;
        }

        info!(
            worker = WORKER_NAME,
            count = running_ids.len(),
            "found running tasks to monitor"
        );

        // Check each running job's status
        for metis_id in &running_ids {
            self.state.reconcile_running_task(metis_id.clone()).await;
        }

        info!(
            worker = WORKER_NAME,
            processed = running_ids.len(),
            "worker iteration completed successfully"
        );

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
        domain::jobs::BundleSpec,
        job_engine::JobStatus,
        store::{Status, Task},
        test_utils::{FailingStore, MockJobEngine, test_state, test_state_with_engine},
    };
    use chrono::Utc;
    use std::{collections::HashMap, sync::Arc};

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
        let task = Task::new(
            "observe".to_string(),
            BundleSpec::None,
            None,
            None,
            HashMap::new(),
            None,
            None,
        );
        let task_id = state
            .add_task(task, Utc::now())
            .await
            .expect("task should be added");
        state
            .mark_task_running(&task_id, Utc::now())
            .await
            .expect("task should be marked running");

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

        assert_eq!(
            state
                .get_task_status(&task_id)
                .await
                .expect("status should exist"),
            Status::Running
        );
    }

    #[tokio::test]
    async fn returns_transient_error_when_store_fails() {
        let mut state = test_state();
        state.set_store_for_tests(Box::new(FailingStore));
        let worker = MonitorRunningJobsWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }
}
