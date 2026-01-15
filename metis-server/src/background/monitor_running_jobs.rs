use crate::{app::AppState, store::Status};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info};

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
        state.reap_orphaned_jobs().await;

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
