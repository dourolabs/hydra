use crate::{app::AppState, store::Status};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Background task that periodically monitors running jobs.
///
/// This function runs in a loop, checking for running tasks every few seconds
/// and updating their status based on the job engine state:
/// 1. Gets all running tasks from the store
/// 2. Checks each job's status in the job engine
/// 3. Updates the store status to Complete or Failed if the job has finished
pub async fn monitor_running_jobs(state: AppState) {
    loop {
        // Check every 5 seconds
        sleep(Duration::from_secs(5)).await;

        // Kill any jobs that are running in the engine but missing from the store
        let job_engine_jobs = match state.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                Vec::new()
            }
        };

        if !job_engine_jobs.is_empty() {
            let store_task_ids = {
                let store = state.store.read().await;
                match store.list_tasks().await {
                    Ok(ids) => Some(ids),
                    Err(err) => {
                        error!(error = %err, "failed to list tasks from store for job reconciliation");
                        None
                    }
                }
            };

            if let Some(store_task_ids) = store_task_ids {
                let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
                let orphaned_jobs: Vec<_> = job_engine_jobs
                    .into_iter()
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
                            info!(metis_id = %job.id, "killed job not present in store");
                        }
                        Err(err) => {
                            warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
                        }
                    }
                }
            }
        }

        // Get running tasks
        let running_ids = {
            let store = state.store.read().await;
            match store.list_tasks_with_status(Status::Running).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list running tasks");
                    continue;
                }
            }
        };

        if running_ids.is_empty() {
            continue;
        }

        info!(count = running_ids.len(), "found running tasks to monitor");

        // Check each running job's status
        for metis_id in running_ids {
            state.reconcile_running_task(metis_id).await;
        }
    }
}
