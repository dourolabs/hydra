use crate::app::AppState;
use chrono::Utc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Background task that periodically runs configured spawners to enqueue new work.
pub async fn run_spawners(state: AppState) {
    let settings = &state.config.background.scheduler.run_spawners;
    let interval_secs = settings.interval_secs.max(1);
    let sleep_duration = Duration::from_secs(interval_secs);
    debug!(
        interval_secs,
        initial_backoff_secs = settings.initial_backoff_secs,
        max_backoff_secs = settings.max_backoff_secs,
        "run_spawners scheduler configured"
    );

    loop {
        sleep(sleep_duration).await;

        if state.spawners.is_empty() {
            continue;
        }

        for spawner in &state.spawners {
            match spawner.spawn(&state).await {
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
                                info!(
                                    spawner = spawner.name(),
                                    metis_id = %metis_id,
                                    "added task produced by spawner"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    spawner = spawner.name(),
                                    error = %err,
                                    "failed to add task from spawner"
                                );
                            }
                        }
                    }
                }
                Err(err) => warn!(spawner = spawner.name(), error = %err, "spawner run failed"),
            }
        }
    }
}
