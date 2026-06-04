//! `ScheduledTriggerWorker`: the production producer of
//! `ActorRef::Trigger` writes and `Trigger -created-> Issue` edges.
//!
//! On each tick (10s default, see `WorkerSchedulerConfig`) the worker
//! reloads the trigger cache via `list_triggers(false)`, computes
//! `next_fire(last_fired_at, schedule)` for each enabled trigger, fires
//! any whose due-slot has elapsed, and records `last_fired_at =
//! scheduled_at` via `Store::record_trigger_fire` regardless of action
//! outcome (the §4.6 scheduling-correctness invariant).
//!
//! See `/designs/triggered-actions.md` §4 / §4.1 / §4.6, and §7 PR 5.

use crate::{
    app::AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::actors::ActorRef,
    domain::triggers::{ActionRun, RenderContext, parse_cron_expression},
    store::Store,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::ActorId;
use hydra_common::TriggerId;
use hydra_common::triggers::{Schedule, Trigger};
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

        let store = self.state.store.inner().clone();

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
            if !trigger.enabled || trigger.deleted {
                continue;
            }
            let Some(scheduled_at) = due_fire(&trigger, now) else {
                continue;
            };

            match fire_trigger(store.as_ref(), &trigger_id, &trigger, scheduled_at).await {
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

            // Record the fire regardless of action outcome (§4.6).
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
/// failures are logged and counted, but the loop continues —
/// `record_trigger_fire` runs regardless (§4.6). Returns the count of
/// successful actions on `Ok`, or the count of failures on `Err` when
/// no action succeeded.
async fn fire_trigger(
    store: &dyn Store,
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
        match action.run(&ctx, store, &actor, trigger_id).await {
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

/// Per §4: return `Some(scheduled_at)` iff the trigger is due-fire at
/// `now`. `Cron` fires the **most recent** slot ≤ `now` and never
/// replays older missed slots; `Once { at }` fires once if `at <= now`.
fn due_fire(trigger: &Trigger, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match &trigger.schedule {
        Schedule::Cron { expression, .. } => {
            let schedule = parse_cron_expression(expression).ok()?;
            // `after()` enumerates slots strictly after the cursor; we
            // want the most recent slot ≤ `now`. Walk forward from
            // `cursor = max(last_fired_at, now - 1d)` (bounded so a
            // never-fired trigger doesn't scan from the epoch) and keep
            // the last slot that is ≤ now.
            let lookback = chrono::Duration::days(1);
            let cursor = match trigger.last_fired_at {
                Some(t) => t,
                None => now - lookback,
            };
            let mut due: Option<DateTime<Utc>> = None;
            // 64 slots cap the work per tick on dense schedules. Per
            // §4 we explicitly do not replay older missed slots, so the
            // most recent is the only one we'd fire anyway.
            for candidate in schedule.after(&cursor).take(8192) {
                if candidate > now {
                    break;
                }
                due = Some(candidate);
            }
            due
        }
        Schedule::Once { at } => {
            if trigger.last_fired_at.is_some() {
                return None;
            }
            if *at <= now { Some(*at) } else { None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::users::Username as ApiUsername;
    use hydra_common::triggers::{Action, CreateIssueAction, Schedule};

    fn sample_trigger(schedule: Schedule, last_fired_at: Option<DateTime<Utc>>) -> Trigger {
        use hydra_common::api::v1::issues::{IssueStatus, IssueType, SessionSettings};
        Trigger::new(
            true,
            schedule,
            vec![Action::CreateIssue(CreateIssueAction::new(
                IssueType::Task,
                "t".to_string(),
                "d".to_string(),
                None,
                Some(IssueStatus::Open),
                SessionSettings::default(),
            ))],
            ApiUsername::from("alice"),
            last_fired_at,
            false,
        )
    }

    #[test]
    fn due_fire_cron_returns_most_recent_slot_within_window() {
        let now: DateTime<Utc> = "2026-06-03T15:04:05Z".parse().unwrap();
        let trigger = sample_trigger(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            None,
        );
        let slot = due_fire(&trigger, now).expect("should be due");
        // The most recent slot at or before now for "every minute" at
        // 15:04:05 is 15:04:00.
        assert_eq!(slot.to_rfc3339(), "2026-06-03T15:04:00+00:00");
    }

    #[test]
    fn due_fire_cron_skips_when_last_fired_at_at_or_after_slot() {
        let now: DateTime<Utc> = "2026-06-03T15:04:05Z".parse().unwrap();
        let trigger = sample_trigger(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            Some("2026-06-03T15:04:00Z".parse().unwrap()),
        );
        assert!(due_fire(&trigger, now).is_none());
    }

    #[test]
    fn due_fire_cron_does_not_replay_after_long_downtime() {
        // Trigger last fired 30 seconds before shutdown; server was down
        // for ~12 minutes; cron is "every minute". Per §4 we fire only
        // the most recent slot, not the missed slots.
        let now: DateTime<Utc> = "2026-06-03T15:12:00Z".parse().unwrap();
        let trigger = sample_trigger(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            Some("2026-06-03T15:00:00Z".parse().unwrap()),
        );
        let slot = due_fire(&trigger, now).expect("should be due");
        assert_eq!(slot.to_rfc3339(), "2026-06-03T15:12:00+00:00");
    }

    #[test]
    fn due_fire_once_at_time_returns_at_when_unfired() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T14:59:50Z".parse().unwrap();
        let trigger = sample_trigger(Schedule::Once { at }, None);
        let slot = due_fire(&trigger, now).unwrap();
        assert_eq!(slot, at);
    }

    #[test]
    fn due_fire_once_skipped_when_already_fired() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T14:59:50Z".parse().unwrap();
        let trigger = sample_trigger(Schedule::Once { at }, Some(at));
        assert!(due_fire(&trigger, now).is_none());
    }

    #[test]
    fn due_fire_once_skipped_when_in_future() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T15:00:30Z".parse().unwrap();
        let trigger = sample_trigger(Schedule::Once { at }, None);
        assert!(due_fire(&trigger, now).is_none());
    }
}
