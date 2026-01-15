use crate::app::AppState;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

/// Background task that periodically runs configured spawners to enqueue new work.
pub async fn run_spawners(state: AppState) {
    loop {
        sleep(Duration::from_secs(3)).await;

        if state.spawners.is_empty() {
            continue;
        }

        for spawner in &state.spawners {
            match spawner.spawn(&state).await {
                Ok(tasks) => {
                    let _ = state.enqueue_spawned_tasks(spawner.name(), tasks).await;
                }
                Err(err) => warn!(spawner = spawner.name(), error = %err, "spawner run failed"),
            }
        }
    }
}
