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

Current defaults: `cascade_issue_status`, `kill_sessions_on_enter`, `github_pr_sync`, `link_artifacts_to_issue`, `link_conversation_to_artifacts`, `spawn_sessions`, `spawn_conversation_sessions`, `start_created_sessions`.

### Self-event filter (avoiding infinite loops)

If your automation subscribes to and also emits events of the **same class**, it must filter self-triggered events to avoid feedback loops. The canonical check is:

```rust
if let ActorRef::Automation { automation_name, .. } = ctx.actor() {
    if automation_name == AUTOMATION_NAME {
        return Ok(());
    }
}
```

If your automation does **not** emit any event of the subscribed class, the self-filter rule does not apply. You may then choose either policy on automation-actor events — early-return (as in [`link_artifacts_to_issue`](../../hydra-server/src/policy/automations/link_artifacts_to_issue.rs)) or intentional unwrap via `on_behalf_of()` (as in [`link_conversation_to_artifacts`](../../hydra-server/src/policy/automations/link_conversation_to_artifacts.rs)) — both are correct.

#### When the actor-name filter is dead code (delegated-actor automations)

The actor-name check only fires when the downstream mutation is itself stamped with `ActorRef::Automation`. If your automation routes side effects through a different actor — e.g. dispatching to a system worker — the canonical block would never match and adding it would be dead code.

[`start_created_sessions`](../../hydra-server/src/policy/automations/start_created_sessions.rs) is the live example: it subscribes to `SessionCreated`/`SessionUpdated` and ultimately emits another `SessionUpdated`, but it does so through `ActorRef::System { worker_name: WORKER_NAME_SESSION_LIFECYCLE, .. }`, not `ActorRef::Automation`. Its self-loop defense is a **structural transition-guard**: it only fires when the new status is `Created` *and* the previous status was not, so the downstream `Created → Pending` transition cannot re-trigger it.

A structural transition-guard is an acceptable alternative to the actor-name check when the canonical mechanism would be inert. Document the guard near the top of `execute`, as `start_created_sessions` does.

### Default automations: subscribe / emit / self-filter strategy

Use this table to decide whether the self-filter rule applies before reading an automation's body.

| Automation | Subscribes to | Emits same class? | Self-filter strategy |
|---|---|---|---|
| `cascade_issue_status` | `IssueUpdated` | Yes (via `upsert_issue`) | Canonical actor-name check |
| `kill_sessions_on_enter` | `IssueUpdated` | No (kills sessions → `SessionUpdated`) | N/A |
| `link_artifacts_to_issue` | `Patch*` / `Document*` | No (only adds relationships) | N/A — early-returns on automation-actor events |
| `link_conversation_to_artifacts` | `Issue*` / `Patch*` / `Document*` | No (only adds relationships) | N/A — intentionally unwraps via `on_behalf_of()` |
| `spawn_sessions` | `Issue*` / `Session*` | Yes | Canonical actor-name check |
| `spawn_conversation_sessions` | `Conversation*` / `Session*` | Yes | Canonical actor-name check |
| `start_created_sessions` | `SessionCreated` / `SessionUpdated` | Yes (via system actor) | Structural transition-guard (see above) |

`github_pr_sync` is also a default automation; its impl lives under [`policy/integrations/`](../../hydra-server/src/policy/integrations/github_pr_sync.rs). It subscribes to `PatchCreated`/`PatchUpdated` and pushes state to GitHub rather than emitting hydra events, so the self-filter rule does not apply.

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
