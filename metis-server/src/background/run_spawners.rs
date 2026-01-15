use crate::{app::AppState, background::WorkerOutcome};
use chrono::Utc;
use tracing::{info, warn};

/// Run configured spawners once to enqueue new work.
pub async fn run_spawners(state: &AppState) -> WorkerOutcome {
    if state.spawners.is_empty() {
        return WorkerOutcome::Idle;
    }

    let mut added = 0usize;
    let mut failed = 0usize;

    for spawner in &state.spawners {
        match spawner.spawn(state).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    continue;
                }

                info!(
                    spawner = spawner.name(),
                    count = tasks.len(),
                    "spawner produced tasks"
                );

                let mut store = state.store.write().await;
                for task in tasks {
                    match store.add_task(task, Utc::now()).await {
                        Ok(metis_id) => {
                            added += 1;
                            info!(
                                spawner = spawner.name(),
                                metis_id = %metis_id,
                                "added task produced by spawner"
                            );
                        }
                        Err(err) => {
                            failed += 1;
                            warn!(
                                spawner = spawner.name(),
                                error = %err,
                                "failed to add task from spawner"
                            );
                        }
                    }
                }
            }
            Err(err) => {
                failed += 1;
                warn!(spawner = spawner.name(), error = %err, "spawner run failed");
            }
        }
    }

    if added == 0 && failed == 0 {
        WorkerOutcome::Idle
    } else {
        WorkerOutcome::Progress {
            processed: added,
            failed,
        }
    }
}
