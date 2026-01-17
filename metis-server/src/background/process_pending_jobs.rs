use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    store::Status,
};
use async_trait::async_trait;
use tracing::{error, info};

/// Scheduled worker that processes pending jobs once per iteration.
///
/// A successful iteration returns `Progress`, empty queues return `Idle`,
/// and store failures map to `TransientError` so the scheduler can back off.
#[derive(Clone)]
pub struct ProcessPendingJobsWorker {
    state: AppState,
}

impl ProcessPendingJobsWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ScheduledWorker for ProcessPendingJobsWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        let pending_ids = {
            let store = self.state.store.read().await;
            match store.list_tasks_with_status(Status::Pending).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list pending tasks");
                    return WorkerOutcome::TransientError {
                        reason: err.to_string(),
                    };
                }
            }
        };

        if pending_ids.is_empty() {
            return WorkerOutcome::Idle;
        }

        info!(count = pending_ids.len(), "found pending tasks to process");

        for metis_id in &pending_ids {
            self.state.start_pending_task(metis_id.clone()).await;
        }

        WorkerOutcome::Progress {
            processed: pending_ids.len(),
            failed: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::{Status, Task},
        test_utils::{FailingStore, test_state},
    };
    use chrono::Utc;
    use metis_common::jobs::BundleSpec;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn returns_idle_when_no_pending_tasks_exist() {
        let state = test_state();
        let worker = ProcessPendingJobsWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn starts_pending_tasks_and_reports_progress() {
        let state = test_state();
        let mut store = state.store.write().await;
        let task = Task {
            prompt: "do work".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: None,
            env_vars: HashMap::new(),
        };
        let first_id = store
            .add_task(task.clone(), Utc::now())
            .await
            .expect("first task should be added");
        let second_id = store
            .add_task(task, Utc::now())
            .await
            .expect("second task should be added");
        drop(store);

        let worker = ProcessPendingJobsWorker::new(state.clone());
        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 2,
                failed: 0
            }
        );

        let store = state.store.read().await;
        assert_eq!(
            store
                .get_status(&first_id)
                .await
                .expect("status should exist"),
            Status::Running
        );
        assert_eq!(
            store
                .get_status(&second_id)
                .await
                .expect("status should exist"),
            Status::Running
        );
    }

    #[tokio::test]
    async fn returns_transient_error_when_store_fails() {
        let mut state = test_state();
        state.store = Arc::new(RwLock::new(Box::new(FailingStore)));
        let worker = ProcessPendingJobsWorker::new(state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }
}
