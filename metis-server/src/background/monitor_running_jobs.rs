use crate::{
    app::{AppState, WORKER_NAME_CLEANUP_ORPHANED_SESSIONS, WORKER_NAME_SESSION_LIFECYCLE},
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::actors::ActorRef,
    store::Status,
};
use async_trait::async_trait;
use metis_common::sessions::SearchSessionsQuery;
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
        let cleanup_actor = ActorRef::System {
            worker_name: WORKER_NAME_CLEANUP_ORPHANED_SESSIONS.into(),
            on_behalf_of: None,
        };
        self.state.cleanup_orphaned_tasks(cleanup_actor).await;

        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![Status::Pending.into(), Status::Running.into()],
        );
        let active_ids: Vec<_> = match self.state.list_tasks_with_query(&query).await {
            Ok(tasks) => tasks.into_iter().map(|(id, _)| id).collect(),
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
        };

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
        let lifecycle_actor = ActorRef::System {
            worker_name: WORKER_NAME_SESSION_LIFECYCLE.into(),
            on_behalf_of: None,
        };
        for metis_id in active_ids {
            self.state
                .reconcile_running_task(metis_id, lifecycle_actor.clone())
                .await;
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
            actors::ActorRef,
            issues::{Issue, IssueStatus, IssueType},
            sessions::BundleSpec,
            users::Username,
        },
        job_engine::JobStatus,
        store::{Session, Status, StoreError},
        test_utils::{
            FailingStore, MockJobEngine, test_state_handles, test_state_with_engine_handles,
            test_state_with_store,
        },
    };
    use chrono::Utc;
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
        let task = Session::new(
            "observe".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        );
        let (task_id, _) = handles
            .store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .expect("task should be added");
        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
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
                .get_session(&task_id)
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
            "Test Title".to_string(),
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
        let (issue_id, _) = handles
            .store
            .add_issue(issue, &ActorRef::test())
            .await
            .unwrap();

        let task = Session::new(
            "spawned task".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            Username::from("test-creator"),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        );
        let (task_id, _) = handles
            .store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        handles
            .store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        let worker = MonitorRunningJobsWorker::new(handles.state);
        worker.run_iteration().await;

        let result = handles.store.get_session(&task_id, false).await;
        assert!(
            matches!(result, Err(StoreError::SessionNotFound(_))),
            "orphaned task should be cleaned up during worker iteration"
        );
    }
}
