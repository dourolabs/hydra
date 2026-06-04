# Errors and logging

## Propagate with `Result`

Use `Result<T, E>` and `?` to propagate errors. Route handlers return
`Result<Json<...>, ApiError>`; CLI entry points return `anyhow::Result`. Map
to a domain error at the boundary that owns the user-visible message — don't
swallow the cause higher up the stack and log it.

```rust
// wrong: error reduced to a log line, caller can't react
fn create(payload: Payload) -> Json<Response> {
    match store.insert(payload) {
        Ok(r) => Json(r),
        Err(e) => { error!(?e); Json(Response::empty()) }
    }
}

// correct
fn create(payload: Payload) -> Result<Json<Response>, ApiError> {
    let r = store.insert(payload)?;
    Ok(Json(r))
}
```

## Panic policy

Panics are for programmer errors only — invariants the compiler can't express
that *must* hold by the time the line runs (e.g. a `Mutex` whose poisoning
would indicate a real bug). Anything that depends on external input,
filesystem state, network, or user data must be a `Result`. In particular:

- No `unwrap()` / `expect()` on user input or I/O. Use `?`.
- No `panic!` to signal "this shouldn't happen but might" — return an error
  with context (`anyhow::bail!` or a typed variant).
- Test code is the exception: `unwrap()` in tests is fine and often clearer
  than threading `Result` through assertions.

## `info!` on every route ingress and decision

Every HTTP handler emits `info!` logs that let an operator trace a request
end-to-end. Log:

- handler name (e.g. `"list_agents invoked"`),
- key identifiers (issue id, repo, user),
- the decision taken or status returned on completion.

```rust
#[tracing::instrument(skip(state))]
pub async fn get_issue(
    State(state): State<AppState>,
    Path(id): Path<IssueId>,
) -> Result<Json<IssueResponse>, ApiError> {
    info!(issue = %id, "get_issue invoked");
    let issue = state.store.get_issue(&id)?;
    info!(issue = %id, "get_issue completed");
    Ok(Json(issue.into()))
}
```

## `info!` on background jobs

Both periodic `ScheduledWorker` impls under `hydra-server/src/background/`
and event-driven `Automation` impls under
`hydra-server/src/policy/automations/` count as background jobs here. Every
invocation logs at `info!` when it starts (job/automation name + key
parameters — for an automation, the triggering event's primary id), at any
fork in the control flow that selects an action (the *decision*), and again
when it finishes (success/failure + a high-level outcome). Automations
additionally log on the skip-self-event branch (bailing when `ctx.actor()`
is the automation itself, to avoid infinite loops). Operators read these
logs to reconstruct what a worker or automation did during a run, so the
start/finish pair is non-negotiable.

```rust
// ScheduledWorker
info!(job = "reconcile_assignments", repo = %repo, "starting");
let outcome = reconcile(&repo).await?;
info!(job = "reconcile_assignments", repo = %repo, ?outcome, "finished");
```

```rust
// policy/automations/* — entry log on dispatch, named alongside the event id
info!(automation = "cascade_issue_status", issue_id = %issue_id, "automation invoked");
```

## Log levels at a glance

| Level | Use for |
|---|---|
| `error!` | Failed operations the operator should investigate. |
| `warn!` | Degraded conditions that the system recovered from. |
| `info!` | Routine route ingress, job lifecycle, decisions. |
| `debug!` / `trace!` | Per-iteration detail; off by default. |
