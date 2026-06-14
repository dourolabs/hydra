//! `AutoArchiveWorker`: per-status periodic auto-archive.
//!
//! Each tick walks every project's statuses and, for any status with
//! `auto_archive_after_seconds = Some(N)`, calls
//! [`ReadOnlyStore::list_stale_issues_for_status`] to find non-archived
//! issues whose latest `updated_at` is older than `now - N seconds`. The
//! worker soft-deletes (= "archives") each match via
//! [`Store::archive_issue`]; the soft-delete filter (`archived = false`)
//! excludes already-archived rows on re-tick, so the loop is naturally
//! idempotent across partial failures and restarts.
//!
//! Per-tick work is capped per status by
//! [`AutoArchiveSchedulerConfig::batch_size`]; the remainder drains over
//! subsequent ticks so newly-lowered thresholds against a large backlog
//! don't blow up tick latency.
//!
//! Archiving an issue never cascades to its children, parents, or
//! dependents — each issue is evaluated independently against its
//! own status's threshold and `Store::archive_issue` is called directly,
//! bypassing any cascade-on-status semantics.

use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::actors::ActorRef,
    store::ReadOnlyStore,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::ProjectId;
use hydra_common::api::v1::projects::StatusKey;
use std::sync::Arc;
use tracing::{error, info, warn};

pub const WORKER_NAME: &str = "auto_archive";

/// Function that returns the current time. Injectable so tests can
/// advance "now" without sleeping. Production code uses `Utc::now`.
pub type NowFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone)]
pub struct AutoArchiveWorker {
    state: AppState,
    batch_size: u32,
    now_fn: NowFn,
}

impl AutoArchiveWorker {
    pub fn new(state: AppState, batch_size: u32, now_fn: NowFn) -> Self {
        Self {
            state,
            batch_size,
            now_fn,
        }
    }
}

#[async_trait]
impl ScheduledWorker for AutoArchiveWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        info!(worker = WORKER_NAME, "worker iteration started");

        if self.batch_size == 0 {
            info!(
                worker = WORKER_NAME,
                "auto_archive batch_size is 0; worker idle"
            );
            return WorkerOutcome::Idle;
        }

        let store = &*self.state.store;

        let projects = match store.list_projects(false).await {
            Ok(p) => p,
            Err(err) => {
                error!(error = %err, worker = WORKER_NAME, "failed to list projects");
                return WorkerOutcome::TransientError {
                    reason: err.to_string(),
                };
            }
        };

        let now = (self.now_fn)();
        let mut processed = 0usize;
        let mut failed = 0usize;
        for (project_id, versioned) in projects {
            for status in &versioned.item.statuses {
                let Some(threshold_seconds) = status.auto_archive_after_seconds else {
                    continue;
                };
                if threshold_seconds < 0 {
                    warn!(
                        worker = WORKER_NAME,
                        project_id = %project_id,
                        status_key = %status.key,
                        threshold_seconds,
                        "ignoring negative auto_archive_after_seconds"
                    );
                    continue;
                }
                let outcome = archive_stale_for_status(
                    store,
                    &project_id,
                    &status.key,
                    threshold_seconds,
                    now,
                    self.batch_size,
                )
                .await;
                processed += outcome.archived;
                failed += outcome.failed;
            }
        }

        if processed == 0 && failed == 0 {
            info!(worker = WORKER_NAME, "no stale issues found; worker idle");
            return WorkerOutcome::Idle;
        }
        info!(
            worker = WORKER_NAME,
            processed, failed, "worker iteration completed"
        );
        WorkerOutcome::Progress { processed, failed }
    }
}

struct PerStatusOutcome {
    archived: usize,
    failed: usize,
}

async fn archive_stale_for_status(
    store: &crate::app::StoreWithEvents,
    project_id: &ProjectId,
    status_key: &StatusKey,
    threshold_seconds: i64,
    now: DateTime<Utc>,
    batch_size: u32,
) -> PerStatusOutcome {
    let stale_ids = match store
        .list_stale_issues_for_status(project_id, status_key, threshold_seconds, now, batch_size)
        .await
    {
        Ok(ids) => ids,
        Err(err) => {
            warn!(
                worker = WORKER_NAME,
                project_id = %project_id,
                status_key = %status_key,
                error = %err,
                "failed to list stale issues for status"
            );
            return PerStatusOutcome {
                archived: 0,
                failed: 1,
            };
        }
    };

    let actor = ActorRef::System {
        worker_name: WORKER_NAME.to_string(),
        on_behalf_of: None,
    };

    let mut archived = 0usize;
    let mut failed = 0usize;
    for issue_id in stale_ids {
        match store
            .delete_issue_with_actor(&issue_id, actor.clone())
            .await
        {
            Ok(_) => {
                info!(
                    worker = WORKER_NAME,
                    issue_id = %issue_id,
                    project_id = %project_id,
                    status_key = %status_key,
                    threshold_seconds,
                    "auto-archived stale issue"
                );
                archived += 1;
            }
            Err(err) => {
                warn!(
                    worker = WORKER_NAME,
                    issue_id = %issue_id,
                    project_id = %project_id,
                    status_key = %status_key,
                    error = %err,
                    "failed to archive stale issue"
                );
                failed += 1;
            }
        }
    }
    PerStatusOutcome { archived, failed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{Issue, IssueDependency, IssueType};
    use crate::domain::projects::default_project_id;
    use crate::domain::users::Username;
    use crate::test_utils::{TestStateHandles, test_state_handles};
    use chrono::Duration;
    use hydra_common::api::v1::projects::StatusKey;
    use hydra_common::test_utils::status::status;

    fn sample_issue(title: &str, status_key: StatusKey) -> Issue {
        Issue::new(
            IssueType::Task,
            title.to_string(),
            "auto-archive test".to_string(),
            Username::from("creator"),
            status_key,
            default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
        )
    }

    /// Set `auto_archive_after_seconds = Some(threshold)` on the given
    /// status of the default project, preserving every other field on
    /// the status row. Goes through `update_status` (not
    /// `update_project`) because the in-memory store's status table is
    /// authoritative: `update_project` ignores the `statuses` field
    /// and replays the index snapshot, so any field-level edit has to
    /// go through `update_status`.
    async fn enable_auto_archive(handles: &TestStateHandles, status_key: &str, threshold: i64) {
        let project_id = default_project_id();
        let versioned = handles
            .store
            .get_project(&project_id, false)
            .await
            .expect("project must exist");
        let mut status = versioned
            .item
            .statuses
            .iter()
            .find(|s| s.key.as_str() == status_key)
            .cloned()
            .unwrap_or_else(|| panic!("status '{status_key}' not found on default project"));
        status.auto_archive_after_seconds = Some(threshold);
        let key = status.key.clone();
        handles
            .store
            .update_status(&project_id, &key, status, &ActorRef::test())
            .await
            .expect("update_status failed");
    }

    fn fixed_clock(now: DateTime<Utc>) -> NowFn {
        Arc::new(move || now)
    }

    #[tokio::test]
    async fn archives_stale_issue_but_not_fresh_one() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;

        let (stale_id, _) = handles
            .store
            .add_issue(sample_issue("stale", status("open")), &ActorRef::test())
            .await
            .unwrap();
        // Sleep briefly so `fresh_id`'s `updated_at` is strictly
        // later than `stale_id`'s. The test then picks
        // `now = fresh.timestamp + threshold_seconds` so the cutoff
        // lands exactly on fresh's timestamp: stale (strictly older)
        // qualifies, fresh (>= cutoff) does not.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (fresh_id, _) = handles
            .store
            .add_issue(sample_issue("fresh", status("open")), &ActorRef::test())
            .await
            .unwrap();

        let stale_versioned = handles.store.get_issue(&stale_id, false).await.unwrap();
        let fresh_versioned = handles.store.get_issue(&fresh_id, false).await.unwrap();
        assert!(
            fresh_versioned.timestamp > stale_versioned.timestamp,
            "test setup: fresh must be added after stale"
        );
        let threshold_seconds = 1i64;
        let now = fresh_versioned.timestamp + Duration::seconds(threshold_seconds);

        let worker = AutoArchiveWorker::new(handles.state.clone(), 100, fixed_clock(now));
        let outcome = worker.run_iteration().await;
        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0,
            }
        );

        let stale = handles.store.get_issue(&stale_id, true).await.unwrap();
        assert!(stale.item.archived, "stale issue should be archived");
        let fresh = handles.store.get_issue(&fresh_id, true).await.unwrap();
        assert!(!fresh.item.archived, "fresh issue must be untouched");
    }

    #[tokio::test]
    async fn no_archive_when_setting_is_none() {
        let handles = test_state_handles();
        // Default project's statuses have `auto_archive_after_seconds
        // = None`. Seed an issue and run far in the future; nothing
        // should be archived.
        let (id, _) = handles
            .store
            .add_issue(sample_issue("aged", status("open")), &ActorRef::test())
            .await
            .unwrap();

        let now = Utc::now() + Duration::days(365);
        let worker = AutoArchiveWorker::new(handles.state.clone(), 100, fixed_clock(now));
        let outcome = worker.run_iteration().await;
        assert_eq!(outcome, WorkerOutcome::Idle);

        let issue = handles.store.get_issue(&id, true).await.unwrap();
        assert!(!issue.item.archived);
    }

    #[tokio::test]
    async fn idempotent_on_re_tick() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;
        let (id, _) = handles
            .store
            .add_issue(sample_issue("aged", status("open")), &ActorRef::test())
            .await
            .unwrap();

        let now = Utc::now() + Duration::days(7);
        let worker = AutoArchiveWorker::new(handles.state.clone(), 100, fixed_clock(now));

        let outcome_1 = worker.run_iteration().await;
        assert_eq!(
            outcome_1,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0,
            }
        );
        let archived = handles.store.get_issue(&id, true).await.unwrap();
        assert!(archived.item.archived);
        let version_after_archive = archived.version;

        let outcome_2 = worker.run_iteration().await;
        assert_eq!(
            outcome_2,
            WorkerOutcome::Idle,
            "re-tick must not touch already-archived rows"
        );
        let still_archived = handles.store.get_issue(&id, true).await.unwrap();
        assert_eq!(
            still_archived.version, version_after_archive,
            "no new version should have been written on re-tick"
        );
    }

    #[tokio::test]
    async fn cascade_safety_archives_parent_only() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;

        let (parent_id, _) = handles
            .store
            .add_issue(sample_issue("parent", status("open")), &ActorRef::test())
            .await
            .unwrap();
        // Sleep so the child's `updated_at` is strictly later than
        // the parent's; the cutoff (= child.timestamp) leaves parent
        // qualifying as stale but not the child.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let mut child = sample_issue("child", status("open"));
        child.dependencies = vec![IssueDependency {
            dependency_type: crate::domain::issues::IssueDependencyType::ChildOf,
            issue_id: parent_id.clone(),
        }];
        let (child_id, _) = handles
            .store
            .add_issue(child, &ActorRef::test())
            .await
            .unwrap();

        let parent_versioned = handles.store.get_issue(&parent_id, false).await.unwrap();
        let child_versioned = handles.store.get_issue(&child_id, false).await.unwrap();
        assert!(
            child_versioned.timestamp > parent_versioned.timestamp,
            "test setup: child must be added after parent"
        );
        let threshold_seconds = 1i64;
        let now = child_versioned.timestamp + Duration::seconds(threshold_seconds);

        let worker = AutoArchiveWorker::new(handles.state.clone(), 100, fixed_clock(now));
        let outcome = worker.run_iteration().await;
        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0,
            }
        );

        let parent = handles.store.get_issue(&parent_id, true).await.unwrap();
        assert!(parent.item.archived, "parent should be archived");
        let child = handles.store.get_issue(&child_id, true).await.unwrap();
        assert!(
            !child.item.archived,
            "child must NOT be cascaded — archive bypasses cascade-on-status"
        );
    }

    #[tokio::test]
    async fn bounded_batch_drains_over_multiple_ticks() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;

        let total = 5usize;
        let mut ids = Vec::with_capacity(total);
        for i in 0..total {
            let title = format!("aged-{i}");
            let (id, _) = handles
                .store
                .add_issue(sample_issue(&title, status("open")), &ActorRef::test())
                .await
                .unwrap();
            ids.push(id);
        }

        let batch_size: u32 = 2;
        let now = Utc::now() + Duration::days(7);
        let worker = AutoArchiveWorker::new(handles.state.clone(), batch_size, fixed_clock(now));

        let mut total_archived = 0usize;
        // Three ticks should drain 2 + 2 + 1 = 5; then an Idle tick.
        for expected in [2usize, 2, 1] {
            let outcome = worker.run_iteration().await;
            match outcome {
                WorkerOutcome::Progress { processed, failed } => {
                    assert_eq!(processed, expected);
                    assert_eq!(failed, 0);
                    total_archived += processed;
                }
                other => panic!("expected Progress, got {other:?}"),
            }
        }
        assert_eq!(total_archived, total);

        let outcome = worker.run_iteration().await;
        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn idle_when_batch_size_zero() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;
        handles
            .store
            .add_issue(sample_issue("aged", status("open")), &ActorRef::test())
            .await
            .unwrap();

        let worker = AutoArchiveWorker::new(
            handles.state.clone(),
            0,
            fixed_clock(Utc::now() + Duration::days(365)),
        );
        let outcome = worker.run_iteration().await;
        assert_eq!(outcome, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn transient_error_when_list_projects_fails() {
        let handles =
            crate::test_utils::test_state_with_store(Arc::new(crate::test_utils::FailingStore));
        let worker = AutoArchiveWorker::new(handles.state, 10, Arc::new(Utc::now));
        let outcome = worker.run_iteration().await;
        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }

    #[tokio::test]
    async fn archive_attributed_to_system_actor() {
        let handles = test_state_handles();
        enable_auto_archive(&handles, "open", 1).await;
        let (id, _) = handles
            .store
            .add_issue(sample_issue("aged", status("open")), &ActorRef::test())
            .await
            .unwrap();

        let now = Utc::now() + Duration::days(365);
        let worker = AutoArchiveWorker::new(handles.state.clone(), 100, fixed_clock(now));
        worker.run_iteration().await;

        let versions = handles.store.get_issue_versions(&id).await.unwrap();
        let latest = versions.last().expect("at least one version");
        assert!(latest.item.archived);
        match latest.actor.as_ref().expect("actor recorded on archive") {
            ActorRef::System { worker_name, .. } => {
                assert_eq!(worker_name, WORKER_NAME);
            }
            other => panic!("expected ActorRef::System, got {other:?}"),
        }
    }
}
