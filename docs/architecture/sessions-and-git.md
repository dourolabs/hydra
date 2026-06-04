# Sessions and git state

Sequential agents on the same issue need to inherit each other's work. Hydra does this with a single tracking branch per work item, owned by the bundle mount.

## Tracking branches

Defined by [`working_branch_name`](../../hydra/src/command/sessions/mounts/bundle.rs):

- `hydra/<issue-id>/head` — used when the session is attached to an issue. Every agent on that issue starts here and pushes back to here.
- `hydra/<session-id>/head` — fallback used when there is no attached issue.

Only one tracking branch exists per session. Patches still live on their own refs (see `hydra patches create`), but the working branch is where the agent's current state is kept between sessions.

## Bundle mount: `setup` and `save`

Worker mounts implement [`Mount`](../../hydra/src/command/sessions/mounts/mod.rs) with a `setup_phase` (runs before the agent) and an optional `save_phase` (runs after). The git lifecycle lives in [`BundleMount`](../../hydra/src/command/sessions/mounts/bundle.rs):

**Setup** (`initialize_tracking_branches`):

1. `clone_repo` → `configure_repo` → `fetch_remote` → `resolve_head_oid`.
2. If `hydra/<issue-id>/head` (or the session fallback) already exists on `origin`, track it locally. Otherwise create the branch at `HEAD` and push it.
3. `checkout_local_branch` so the agent works directly on the tracking branch.

**Save** (`finalize_task_run`):

1. If the work directory has uncommitted changes, `stage_all_changes` + `commit_changes` with a `"Hydra worker auto-commit"` message. No-op when the tree is clean.
2. Push semantics depend on attachment — see below.

`Bundle::None` has no save phase. `Bundle::Unknown` is rejected.

## Push semantics: issue-attached vs. session-attached

This is the load-bearing rule that makes patches the source of truth:

```rust
// correct: issue-attached sessions never push their working branch
if issue_id.is_some() {
    return Ok(());
}
let working_branch = working_branch_name(None, task_id);
push_branch(repo_root, &working_branch, github_token, true)?;
```

- **Issue-attached** (`hydra/<issue-id>/head`): the auto-commit stays **local**. The remote ref only advances when the agent calls `hydra patches create` or `hydra patches update`. Anything not turned into a patch is discarded when the worker is reaped — this is an accepted trade-off so `hydra/<issue-id>/head` on origin always corresponds to a submitted patch.
- **Session-attached** (`hydra/<session-id>/head`): the auto-commit is force-pushed at save time so the remote mirrors local state. There's no patch flow here; the branch is the artifact.

## How sequential agents pick up prior work

When a follow-up session spawns for the same issue (e.g. after a review, retry, or parent re-plan), its bundle mount sees that `hydra/<issue-id>/head` already exists on `origin`, tracks the remote copy, and checks it out. The new agent starts on top of the prior patch state.

Because issue-attached sessions only advance the remote via patch operations, any "in-progress" work that did not turn into a patch is lost between agents. Agents that need to resume mid-task should submit a draft patch before exiting; relying on the local auto-commit is unsafe across worker boundaries.

## Branch cleanup

Stale `hydra/<id>/head` branches on origin are pruned by the [`cleanup_branches`](../../hydra-server/src/background/cleanup_branches.rs) background worker, capped at `MAX_DELETIONS_PER_ITERATION` per run to stay inside GitHub's API limits.
