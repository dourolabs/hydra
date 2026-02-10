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

        // Clean up tasks whose spawned_from issue has been deleted
        self.state.cleanup_orphaned_tasks().await;

        // Clean up completed/failed jobs that have exceeded the grace period
        self.state.cleanup_completed_jobs().await;

        let mut active_ids = Vec::new();
        for status in [Status::Pending, Status::Running] {
            match self.state.list_tasks_with_status(status).await {
                Ok(ids) => active_ids.extend(ids),
                Err(err) => {
                    error!(error = %err, "failed to list active tasks");
                    info!(
                        worker = WORKER_NAME,
                        "worker iteration completed with transient error"
                    );
                    return WorkerOutcome::TransientError {
                        reason: err.to_string(),
                    };
                }
            }
        }

        if active_ids.is_empty() {
            info!(worker = WORKER_NAME, "no active tasks found; worker idle");
            return WorkerOutcome::Idle;
        }

        let active_count = active_ids.len();
        info!(
            worker = WORKER_NAME,
            count = active_count,
            "found active tasks to monitor"
        );

        // Check each active job's status
        for metis_id in active_ids {
            self.state.reconcile_running_task(metis_id).await;
        }

        info!(
            worker = WORKER_NAME,
            processed = active_count,
            "worker iteration completed successfully"
        );

        WorkerOutcome::Progress {
            processed: active_count,
            failed: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            issues::{Issue, IssueStatus, IssueType},
            jobs::BundleSpec,
            users::Username,
        },
        job_engine::{JobEngine, JobStatus},
        store::{Status, StoreError, Task},
        test_utils::{
            FailingStore, MockJobEngine, test_state_handles, test_state_with_engine_handles,
            test_state_with_store,
        },
    };
    use chrono::{Duration, Utc};
    use std::{collections::HashMap, sync::Arc};

    #[tokio::test]
    async fn returns_idle_when_no_running_tasks_exist() {
        let handles = test_state_handles();
        let worker = MonitorRunningJobsWorker::new(handles.state);

        let outcome = worker.run_iteration().await;

        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn reconciles_running_jobs_and_reports_progress() {
        let engine = Arc::new(MockJobEngine::new());
        let handles = test_state_with_engine_handles(engine.clone());
        let task = Task::new(
            "observe".to_string(),
            BundleSpec::None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );
        let (task_id, _) = handles
            .store
            .add_task(task, Utc::now())
            .await
            .expect("task should be added");
        handles
            .state
            .transition_task_to_pending(&task_id)
            .await
            .expect("task should be marked pending");

        engine.insert_job(&task_id, JobStatus::Running).await;

        let worker = MonitorRunningJobsWorker::new(handles.state.clone());
        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        );

        assert_eq!(
            handles
                .state
                .get_task(&task_id)
                .await
                .expect("task should exist")
                .status,
            Status::Running
        );
    }

    #[tokio::test]
    async fn returns_transient_error_when_store_fails() {
        let handles = test_state_with_store(Arc::new(FailingStore));
        let worker = MonitorRunningJobsWorker::new(handles.state);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }

    #[tokio::test]
    async fn run_iteration_cleans_up_orphaned_tasks() {
        let engine = Arc::new(MockJobEngine::new());
        let handles = test_state_with_engine_handles(engine.clone());

        let issue = Issue::new(
            IssueType::Task,
            "parent issue".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = handles.store.add_issue(issue).await.unwrap();

        let task = Task::new(
            "spawned task".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );
        let (task_id, _) = handles.store.add_task(task, Utc::now()).await.unwrap();

        handles.store.delete_issue(&issue_id).await.unwrap();

        let worker = MonitorRunningJobsWorker::new(handles.state);
        worker.run_iteration().await;

        let result = handles.store.get_task(&task_id, false).await;
        assert!(
            matches!(result, Err(StoreError::TaskNotFound(_))),
            "orphaned task should be cleaned up during worker iteration"
        );
    }

    #[tokio::test]
    async fn run_iteration_cleans_up_stale_completed_jobs() {
        let engine = Arc::new(MockJobEngine::new());
        let handles = test_state_with_engine_handles(engine.clone());

        // Create tasks in the store so reap_orphaned_jobs() doesn't kill them
        let stale_task = Task::new(
            "stale task".to_string(),
            BundleSpec::None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );
        let (stale_id, _) = handles
            .store
            .add_task(stale_task, Utc::now())
            .await
            .expect("stale task should be added");

        let recent_task = Task::new(
            "recent task".to_string(),
            BundleSpec::None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        );
        let (recent_id, _) = handles
            .store
            .add_task(recent_task, Utc::now())
            .await
            .expect("recent task should be added");

        // Stale completed job: completion_time 10 minutes ago
        engine
            .insert_job_with_metadata(
                &stale_id,
                JobStatus::Complete,
                Some(Utc::now() - Duration::minutes(10)),
                None,
            )
            .await;

        // Recent completed job: completion_time 1 minute ago
        engine
            .insert_job_with_metadata(
                &recent_id,
                JobStatus::Complete,
                Some(Utc::now() - Duration::minutes(1)),
                None,
            )
            .await;

        let worker = MonitorRunningJobsWorker::new(handles.state);
        worker.run_iteration().await;

        // Stale job should have been cleaned up (killed)
        let stale_status = engine
            .find_job_by_metis_id(&stale_id)
            .await
            .expect("stale job should exist")
            .status;
        assert_eq!(
            stale_status,
            JobStatus::Failed,
            "stale completed job should be cleaned up during worker iteration"
        );

        // Recent job should be left alone
        let recent_status = engine
            .find_job_by_metis_id(&recent_id)
            .await
            .expect("recent job should exist")
            .status;
        assert_eq!(
            recent_status,
            JobStatus::Complete,
            "recent completed job should not be cleaned up"
        );
    }
}
