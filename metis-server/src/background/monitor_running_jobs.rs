use crate::{app::AppState, background::WorkerOutcome, store::Status};
use std::collections::HashSet;
use tracing::{error, info, warn};

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
        let store_task_ids = {
            let store = state.store.read().await;
            match store.list_tasks().await {
                Ok(ids) => Ok(ids),
                Err(err) => {
                    error!(
                        error = %err,
                        "failed to list tasks from store for job reconciliation"
                    );
                    Err(())
                }
            }
        };

        if let Ok(store_task_ids) = store_task_ids {
            let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
            let orphaned_jobs: Vec<_> = job_engine_jobs
                .iter()
                .filter(|job| !store_task_set.contains(&job.id))
                .collect();

            if !orphaned_jobs.is_empty() {
                info!(
                    count = orphaned_jobs.len(),
                    "killing jobs present in engine but missing from store"
                );
            }

            for job in orphaned_jobs {
                match state.job_engine.kill_job(&job.id).await {
                    Ok(()) => {
                        processed += 1;
                        info!(metis_id = %job.id, "killed job not present in store");
                    }
                    Err(err) => {
                        failed += 1;
                        warn!(
                            metis_id = %job.id,
                            error = %err,
                            "failed to kill job not present in store"
                        );
                    }
                }
            }
        } else {
            return WorkerOutcome::TransientError {
                reason: "list_tasks_failed".to_string(),
            };
        }
    }

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

    if running_ids.is_empty() && processed == 0 && failed == 0 {
        return WorkerOutcome::Idle;
    }

    if !running_ids.is_empty() {
        info!(count = running_ids.len(), "found running tasks to monitor");
    }

    for metis_id in running_ids {
        state.reconcile_running_task(metis_id).await;
        processed += 1;
    }

    if processed == 0 && failed == 0 {
        WorkerOutcome::Idle
    } else {
        WorkerOutcome::Progress { processed, failed }
    }
}
