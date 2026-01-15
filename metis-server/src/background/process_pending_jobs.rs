use crate::{app::AppState, background::WorkerOutcome, store::Status};
use tracing::{error, info};

/// Process pending jobs once.
pub async fn process_pending_jobs(state: &AppState) -> WorkerOutcome {
    let pending_ids = {
        let store = state.store.read().await;
        match store.list_tasks_with_status(Status::Pending).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list pending tasks");
                return WorkerOutcome::TransientError {
                    reason: "list_pending_failed".to_string(),
                };
            }
        }
    };

    if pending_ids.is_empty() {
        return WorkerOutcome::Idle;
    }

    let pending_count = pending_ids.len();
    info!(count = pending_count, "found pending tasks to process");

    for metis_id in pending_ids {
        state.start_pending_task(metis_id).await;
    }

    WorkerOutcome::Progress {
        processed: pending_count,
        failed: 0,
    }
}
