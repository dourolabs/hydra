# Automations vs. background workers

Two ways to run server-side logic outside of an HTTP request. Pick by the trigger, not by the action.

## When to use each

| Use an **automation** when… | Use a **background worker** when… |
|---|---|
| The work reacts to a specific entity mutation (issue/session/conversation/patch event). | The work is periodic or polls an external surface that doesn't emit events. |
| You want the response to be near-immediate after the event commits. | A small amount of latency is fine and bounded by an interval. |
| You only need read access to the current store + the mutation payload. | You need to scan broad sets of entities, talk to GitHub, prune resources, etc. |

If the trigger is "X just happened in hydra," default to an automation. If the trigger is "every N seconds" or "what changed outside hydra since last time," it's a worker.

## Automations

Trait: [`Automation`](../../hydra-server/src/policy/mod.rs). Defaults live in [`hydra-server/src/policy/automations/`](../../hydra-server/src/policy/automations/). The runner is [`PolicyEngine::run_automations`](../../hydra-server/src/policy/mod.rs) — it fires after a successful store mutation, dispatches the resulting `ServerEvent` to any automation whose `EventFilter` matches, and logs (rather than rethrows) errors so the original mutation still succeeds.

Wiring a new one takes three steps; skipping any of them leaves it silently inert:

1. Implement `Automation` on a struct with a stable `name()`.
2. Register a factory under that name in [`build_default_registry`](../../hydra-server/src/policy/registry.rs).
3. Add the name to [`default_policy_config`](../../hydra-server/src/app/app_state.rs) or to the operator's `policies.automations` config. Registration alone does **not** activate; activation comes from the `PolicyList`.

To avoid infinite loops, automations should skip events triggered by themselves — check `ctx.actor()` for an `ActorRef::Automation` with the matching `automation_name`.

Current defaults: `cascade_issue_status`, `kill_tasks_on_issue_failure`, `github_pr_sync`, `link_artifacts_to_issue`, `link_conversation_to_artifacts`, `spawn_sessions`, `spawn_conversation_sessions`, `start_created_sessions`.

## Background workers

Trait: [`ScheduledWorker`](../../hydra-server/src/background/scheduler.rs). Each worker exposes `run_iteration` returning a `WorkerOutcome` (`Idle`, `Progress { processed, failed }`, or `TransientError { reason }`). The [`BackgroundScheduler`](../../hydra-server/src/background/scheduler.rs) calls each worker on its configured interval, applies exponential backoff on transient errors, and shuts down cleanly via a `watch::channel`.

Workers are wired in [`start_background_scheduler`](../../hydra-server/src/background/scheduler.rs):

- [`monitor_running_sessions`](../../hydra-server/src/background/monitor_running_sessions.rs) — reap orphaned sessions, drive lifecycle for running tasks.
- [`github_poller`](../../hydra-server/src/policy/integrations/github_pr_poller.rs) — pull PR state for repos that need polling.
- [`cleanup_branches`](../../hydra-server/src/background/cleanup_branches.rs) — delete stale `hydra/<id>/head` refs from GitHub.

Intervals come from `WorkerSchedulerConfig` (see `hydra-server/config.yaml.sample`). Backoff uses `initial_backoff_secs` doubling to `max_backoff_secs`.

## Choosing between them

Spawning sessions used to be a poller; it is now the event-driven [`spawn_sessions`](../../hydra-server/src/policy/automations/spawn_sessions.rs) automation that reacts to `IssueCreated`, `IssueUpdated`, and `SessionUpdated`. If you're tempted to add a new "every N seconds, scan all issues for X" worker, check first whether the underlying signal is already on the event bus — an automation is almost always the right answer.

Conversely, periodic GitHub fetches and stale-branch cleanup will never be event-driven because hydra is not the source of those changes. Those belong as workers.
