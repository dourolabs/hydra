# AppState Restructuring Design

## Problem Statement

`metis-server/src/app/app_state.rs` is a 4182-line file containing the entire `AppState` struct, 7 error types, all public methods (~45 methods on `impl AppState`), a standalone `issue_ready()` function, and ~2100 lines of tests. The file mixes unrelated concerns — issue management, document operations, task lifecycle, job engine reconciliation, merge queue operations, agent queue management, user/auth management, and repository CRUD — all in a single flat implementation block. This makes the file difficult to navigate and understand.

## Goals

- Break `app_state.rs` into multiple files organized by logical domain, so each file is small enough to understand in isolation.
- Do **not** change the `AppState` struct definition (same fields, same type). The struct stays in one place.
- Do **not** change any public API or behavior. Route handlers, background workers, and automations should continue calling the same methods with the same signatures.
- Move tests alongside the code they exercise.

## Non-Goals

- Refactoring `AppState` into multiple structs, introducing new abstractions, or changing the struct's fields.
- Changing method signatures, error types, or return types.
- Modifying `event_bus.rs`, `resolved_task.rs`, or `mod.rs` (beyond re-exports).

## Proposed Approach

Split the `impl AppState` block and its associated error types into domain-focused files, using Rust's ability to have multiple `impl` blocks for the same type across files within the same module. The `app_state.rs` file keeps the struct definition and constructor; each new file adds an `impl AppState` block with methods for a specific domain.

### New File Layout

```
metis-server/src/app/
├── mod.rs                       # (unchanged) Re-exports, ServiceState, errors, helpers
├── app_state.rs                 # Struct definition + constructor + policy engine builder + accessors
├── issues.rs                    # Issue operations (upsert, delete, list, readiness, todo items)
├── documents.rs                 # Document operations (upsert, delete, list, get)
├── patches.rs                   # Patch operations (upsert, delete, list, get)
├── jobs.rs                      # Task/job creation, status transitions, reconciliation, cleanup
├── repositories.rs              # Repository CRUD
├── agents.rs                    # Agent queue CRUD
├── users.rs                     # Login, actor creation, user/token management
├── merge_queue.rs               # Merge queue operations
├── event_bus.rs                 # (unchanged)
├── resolved_task.rs             # (unchanged)
```

### File Contents

#### `app_state.rs` (slimmed down — struct + constructor + infrastructure)

Keeps:
- `AppState` struct definition (lines 47-55)
- `AppState::new()` constructor
- `AppState::build_policy_engine()`
- `AppState::with_policy_engine()` (test helper)
- `AppState::subscribe()`, `event_bus()`, `policy_engine()`, `store()` — infrastructure accessors

Everything else moves out.

#### `issues.rs` — Issue Lifecycle

Methods:
- `get_issue()`, `get_issue_versions()`, `search_issue_graph()`
- `list_issues()`, `list_issues_with_query()`
- `upsert_issue()`, `delete_issue()`
- `is_issue_ready()`
- `get_issue_children()`
- `add_todo_item()`, `replace_todo_list()`, `set_todo_item_status()`

Error types:
- `UpsertIssueError`
- `UpdateTodoListError`

Free function:
- `issue_ready()` (private helper)

Tests: all issue-related tests (`open_issue_ready_when_not_blocked`, `closing_issue_requires_closed_blockers`, `create_issue_inherits_creator_from_parent`, etc.)

#### `documents.rs` — Document Operations

Methods:
- `upsert_document()`, `get_document()`, `get_document_versions()`
- `list_documents()`, `delete_document()`, `get_documents_by_path()`

Error type:
- `UpsertDocumentError`

Tests: `upsert_document_allows_normal_path`, `upsert_document_allows_no_path`

#### `patches.rs` — Patch Operations

Methods:
- `upsert_patch()`, `get_patch()`, `get_patch_versions()`
- `list_patches()`, `list_patches_with_query()`, `delete_patch()`

Error type:
- `UpsertPatchError`

Tests: `upsert_patch_sync_github_*`, `upsert_patch_rejects_duplicate_branch_name_on_create`, etc.

#### `jobs.rs` — Task/Job Lifecycle

Methods:
- `create_job()`, `set_job_status()`
- `get_task()`, `get_task_versions()`, `get_tasks_for_issue()`
- `list_tasks()`, `list_tasks_with_query()`
- `add_task()`, `get_status_log()`, `get_status_logs()`
- `start_pending_task()`
- `transition_task_to_pending()`, `transition_task_to_running()`, `transition_task_to_completion()`, `transition_task_to_completion_with_actor()`
- `reap_orphaned_jobs()`, `cleanup_orphaned_tasks()`, `reconcile_running_task()`
- `apply_job_settings_defaults()`

Error types:
- `CreateJobError`
- `SetJobStatusError`

Tests: `start_pending_task_*`, `reap_orphaned_jobs_*`, `reconcile_running_task_*`, `cleanup_orphaned_tasks_*`, `apply_job_settings_defaults_*`

#### `repositories.rs` — Repository CRUD

Methods:
- `list_repositories()`, `create_repository()`, `update_repository()`, `delete_repository()`
- `repository_from_store()` (used by merge queue and spawner)

No new error types — `RepositoryError` is already in `mod.rs`.

#### `agents.rs` — Agent Queue Management

Methods:
- `list_agent_configs()`, `get_agent_config()`
- `create_agent()`, `update_agent()`, `delete_agent()`
- `agent_queues()`

Error type:
- `AgentError`

#### `users.rs` — Auth & User Management

Methods:
- `login_with_github_token()`
- `create_actor_for_github_token()` (private)
- `create_actor_for_task()` (private — also called from `start_pending_task`, so this may need `pub(crate)` visibility)
- `get_actor()`, `get_user()`, `set_user_github_token()`

Error type:
- `LoginError`

Tests: `login_persists_user_and_actor`, `login_returns_error_for_invalid_token`

#### `merge_queue.rs` — Merge Queue Operations

Methods:
- `merge_queue()`, `enqueue_merge_queue_patch()`
- `load_patch()` (private helper)

No new error types — `MergeQueueError` is already in `mod.rs`.

### Visibility Considerations

Some private methods are called across domain boundaries:
- `create_actor_for_task()` — defined in users.rs, called from jobs.rs (`start_pending_task`). Change to `pub(crate)`.
- `load_patch()` — defined in merge_queue.rs, only used internally. Stays private.
- `transition_task_to_completion_with_actor()` — defined in jobs.rs, called only from within jobs.rs. Stays private.

### `mod.rs` Updates

The `mod.rs` file gains new module declarations:

```rust
mod app_state;
mod issues;
mod documents;
mod patches;
mod jobs;
mod repositories;
mod agents;
mod users;
mod merge_queue;
pub mod event_bus;
mod resolved_task;
```

The existing `pub use app_state::{...}` line is updated to re-export error types from their new locations. Alternatively, error types can stay exported from the file that defines them and the `pub use` in `mod.rs` collects them for external consumers. Either approach works; the key constraint is that existing `use crate::app::{AppState, UpsertIssueError, ...}` imports continue to compile.

### Test Organization

Each new file contains its own `#[cfg(test)] mod tests` block with the tests that exercise its methods. Shared test utilities (`sample_task()`, `task_for_issue()`, `issue_with_status()`, `start_test_automation_runner()`, `wait_for_automations()`, `poll_until()`, `state_with_default_model()`, `github_pull_request_response()`) move to a shared location — either the existing `test_utils` module or a new `app::test_helpers` module visible under `#[cfg(test)]`.

## Risks and Open Questions

1. **Cross-domain method calls**: Some methods span domains (e.g., `start_pending_task` in jobs.rs calls `create_actor_for_task` from users.rs). This is addressed by making such methods `pub(crate)` — no architectural issue, just a visibility change.

2. **Error type re-exports**: The current `mod.rs` re-exports error types from `app_state`. After the split, these re-exports need updating. This is mechanical.

3. **Test helper placement**: The tests use ~7 shared helper functions. These should go in a dedicated test helpers submodule to avoid duplication.

## Key Changes Summary

| File | Approximate Lines | Content |
|------|-------------------|---------|
| `app_state.rs` | ~100 | Struct + constructor + accessors |
| `issues.rs` | ~500 method + ~800 test | Issue CRUD, readiness, todos |
| `documents.rs` | ~120 method + ~30 test | Document CRUD |
| `patches.rs` | ~100 method + ~350 test | Patch CRUD |
| `jobs.rs` | ~650 method + ~400 test | Job lifecycle, reconciliation |
| `repositories.rs` | ~100 | Repository CRUD |
| `agents.rs` | ~80 | Agent queue management |
| `users.rs` | ~150 method + ~50 test | Auth, login, user management |
| `merge_queue.rs` | ~70 | Merge queue operations |

## Acceptance Criteria

- All existing tests pass without modification to test logic (only `use` paths may change).
- `cargo check --workspace` compiles cleanly.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- No public API changes — all route handlers, background workers, and automations compile without changes.
- Each file in `metis-server/src/app/` is under ~1500 lines including tests.
