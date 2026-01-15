use crate::{app::AppState, store::Status};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info};

/// Background task that periodically processes pending jobs.
///
/// This function runs in a loop, checking for pending tasks every few seconds
/// and starting them by:
/// 1. Setting their status to Running
/// 2. Creating the Kubernetes job via the job engine
pub async fn process_pending_jobs(state: AppState) {
    let settings = &state.config.background.scheduler.process_pending_jobs;
    let interval_secs = settings.interval_secs.max(1);
    let sleep_duration = Duration::from_secs(interval_secs);
    debug!(
        interval_secs,
        initial_backoff_secs = settings.initial_backoff_secs,
        max_backoff_secs = settings.max_backoff_secs,
        "process_pending_jobs scheduler configured"
    );

    loop {
        sleep(sleep_duration).await;

        // Get pending tasks
        let pending_ids = {
            let store = state.store.read().await;
            match store.list_tasks_with_status(Status::Pending).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list pending tasks");
                    continue;
                }
            }
        };

        if pending_ids.is_empty() {
            continue;
        }

        info!(count = pending_ids.len(), "found pending tasks to process");

        // Process each pending task
        for metis_id in pending_ids {
            state.start_pending_task(metis_id).await;
        }
    }
}
