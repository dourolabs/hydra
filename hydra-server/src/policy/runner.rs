use crate::app::AppState;
use crate::app::event_bus::ServerEvent;
use crate::policy::context::AutomationContext;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::broadcast;
use tokio::sync::watch;

/// Thread-local depth counter for re-entrancy protection.
///
/// Automations that call `AppState` methods will trigger new events on the bus.
/// The runner increments a depth counter before processing each event and
/// decrements it afterwards. Events arriving when the depth exceeds
/// `MAX_RECURSION_DEPTH` are silently dropped to prevent infinite loops.
static AUTOMATION_DEPTH: AtomicU32 = AtomicU32::new(0);

const MAX_RECURSION_DEPTH: u32 = 10;

/// Spawn the automation runner as a background tokio task.
///
/// The runner subscribes to the event bus and sequentially runs all matching
/// automations for each event. It runs until the shutdown signal is received.
///
/// Returns a `JoinHandle` that can be awaited for graceful shutdown.
pub fn spawn_automation_runner(
    state: AppState,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let rx = state.subscribe();
    tokio::spawn(run_automation_loop(state, rx, shutdown_rx))
}

async fn run_automation_loop(
    state: AppState,
    mut rx: broadcast::Receiver<ServerEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    tracing::info!("automation runner started");

    loop {
        let event = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                tracing::info!("automation runner shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            skipped = n,
                            "automation runner lagged behind event bus; skipped events"
                        );
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("event bus closed; automation runner exiting");
                        break;
                    }
                }
            }
        };

        let depth = AUTOMATION_DEPTH.fetch_add(1, Ordering::SeqCst);
        if depth >= MAX_RECURSION_DEPTH {
            AUTOMATION_DEPTH.fetch_sub(1, Ordering::SeqCst);
            tracing::warn!(
                depth,
                max = MAX_RECURSION_DEPTH,
                "automation recursion depth exceeded; dropping event"
            );
            continue;
        }

        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        state.policy_engine().run_automations(&ctx).await;

        AUTOMATION_DEPTH.fetch_sub(1, Ordering::SeqCst);
    }
}
