use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    store::Status,
};
use async_trait::async_trait;
use metis_common::jobs::SearchJobsQuery;
use tracing::{error, info};

const WORKER_NAME: &str = "process_pending_jobs";

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
        info!(worker = WORKER_NAME, "worker iteration started");
        let query = SearchJobsQuery::new(None, None, None, Some(Status::Created.into()));
        let pending_ids: Vec<_> = match self.state.list_tasks_with_query(&query).await {
            Ok(tasks) => tasks.into_iter().map(|(id, _)| id).collect(),
            Err(err) => {
                error!(error = %err, "failed to list created tasks");
                info!(
                    worker = WORKER_NAME,
                    "worker iteration completed with transient error"
                );
                return WorkerOutcome::TransientError {
                    reason: err.to_string(),
                };
            }
        };

        if pending_ids.is_empty() {
            info!(worker = WORKER_NAME, "no created tasks found; worker idle");
            return WorkerOutcome::Idle;
        }

        info!(
            worker = WORKER_NAME,
            count = pending_ids.len(),
            "found created tasks to process"
        );

        for metis_id in &pending_ids {
            self.state.start_pending_task(metis_id.clone()).await;
        }

        info!(
            worker = WORKER_NAME,
            processed = pending_ids.len(),
            "worker iteration completed successfully"
        );

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
        domain::jobs::BundleSpec,
        store::{Status, Task},
        test_utils::{FailingStore, test_state, test_state_with_store},
    };
    use chrono::Utc;
    use std::{collections::HashMap, sync::Arc};

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
        let task = Task::new(
            "do work".to_string(),
            BundleSpec::None,
            None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );
        let first_id = state
            .add_task(task.clone(), Utc::now())
            .await
            .expect("first task should be added");
        let second_id = state
            .add_task(task, Utc::now())
            .await
            .expect("second task should be added");

        let worker = ProcessPendingJobsWorker::new(state.clone());
        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 2,
                failed: 0
            }
        );

        assert_eq!(
            state
                .get_task(&first_id)
                .await
                .expect("task should exist")
                .status,
            Status::Pending
        );
        assert_eq!(
            state
                .get_task(&second_id)
                .await
                .expect("task should exist")
                .status,
            Status::Pending
        );
    }

    #[tokio::test]
    async fn returns_transient_error_when_store_fails() {
        let handles = test_state_with_store(Arc::new(FailingStore));
        let worker = ProcessPendingJobsWorker::new(handles.state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }
}
