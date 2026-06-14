//! `ScheduledTriggerWorker`: the production producer of
//! `ActorRef::Trigger` writes and `Trigger -created-> Issue` edges.
//!
//! On each tick (10s default, see `WorkerSchedulerConfig`) the worker
//! reloads the trigger cache via `list_triggers(false)`, asks each
//! `Schedule::get_fire_candidate(last_fired_at, now)` whether the
//! trigger is due, fires any that are, and records `last_fired_at =
//! scheduled_at` via `Store::record_trigger_fire` regardless of action
//! outcome — the scheduling-correctness invariant that makes the worker
//! self-correcting across restarts.

use crate::{
    app::{AppState, StoreWithEvents},
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::actors::ActorRef,
    domain::triggers::{ActionRun, RenderContext, ScheduleFiring},
    store::ReadOnlyStore,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::ActorId;
use hydra_common::TriggerId;
use hydra_common::triggers::Trigger;
use tracing::{error, info, warn};

pub const WORKER_NAME: &str = "scheduled_triggers";

#[derive(Clone)]
pub struct ScheduledTriggerWorker {
    state: AppState,
}

impl ScheduledTriggerWorker {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ScheduledWorker for ScheduledTriggerWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        info!(worker = WORKER_NAME, "worker iteration started");

        let store = &*self.state.store;

        let triggers = match store.list_triggers(false).await {
            Ok(t) => t,
            Err(err) => {
                error!(error = %err, worker = WORKER_NAME, "failed to list triggers");
                return WorkerOutcome::TransientError {
                    reason: err.to_string(),
                };
            }
        };

        let now = Utc::now();
        let mut processed = 0usize;
        let mut failed = 0usize;
        for (trigger_id, versioned) in triggers {
            let trigger = versioned.item;
            if !trigger.enabled || trigger.archived {
                continue;
            }
            let Some(scheduled_at) = trigger
                .schedule
                .get_fire_candidate(trigger.last_fired_at, now)
            else {
                continue;
            };

            match fire_trigger(store, &trigger_id, &trigger, scheduled_at).await {
                Ok(succeeded) => {
                    processed += succeeded;
                    info!(
                        worker = WORKER_NAME,
                        trigger_id = %trigger_id,
                        scheduled_at = %scheduled_at,
                        actions = succeeded,
                        "trigger fired"
                    );
                }
                Err(count) => {
                    failed += count;
                }
            }

            // Always record the fire; this is what makes the worker
            // self-correcting across restarts (once `last_fired_at >=
            // s`, slot `s` is skipped on rehydrate).
            if let Err(err) = store.record_trigger_fire(&trigger_id, scheduled_at).await {
                warn!(
                    worker = WORKER_NAME,
                    trigger_id = %trigger_id,
                    error = %err,
                    "failed to record_trigger_fire; slot may refire next tick"
                );
            }
        }

        if processed == 0 && failed == 0 {
            info!(worker = WORKER_NAME, "no due triggers; worker idle");
            return WorkerOutcome::Idle;
        }
        info!(
            worker = WORKER_NAME,
            processed, failed, "worker iteration completed"
        );
        WorkerOutcome::Progress { processed, failed }
    }
}

/// Run every action in the trigger's `actions` list. Per-action
/// failures are logged and counted, but the loop continues — the worker
/// records the fire regardless. Returns the count of successful actions
/// on `Ok`, or the count of failures on `Err` when no action succeeded.
async fn fire_trigger(
    store: &StoreWithEvents,
    trigger_id: &TriggerId,
    trigger: &Trigger,
    scheduled_at: DateTime<Utc>,
) -> Result<usize, usize> {
    let actor = ActorRef::Trigger {
        trigger_id: trigger_id.clone(),
        on_behalf_of: Some(ActorId::User(trigger.creator.clone())),
    };
    let ctx = RenderContext::new(Utc::now(), scheduled_at, trigger_id.clone());

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for action in &trigger.actions {
        match action
            .run(&ctx, store, &actor, &trigger.creator, trigger_id)
            .await
        {
            Ok(_) => succeeded += 1,
            Err(err) => {
                failed += 1;
                error!(
                    worker = WORKER_NAME,
                    trigger_id = %trigger_id,
                    error = %err,
                    "trigger action failed"
                );
            }
        }
    }
    if succeeded == 0 && failed > 0 {
        Err(failed)
    } else {
        Ok(succeeded)
    }
}
