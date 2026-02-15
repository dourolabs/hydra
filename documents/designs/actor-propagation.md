# Design: Universal Actor Propagation

## Problem Statement

Today, Metis tracks _who_ performed an action inconsistently. The `StoreWithEvents` layer already accepts an `actor: Option<String>` parameter and carries it through `MutationPayload` into the event bus, but:

1. **The actor is optional everywhere.** Many internal call sites pass `None` (background workers, automations, some AppState helper methods). This means events and store versions often lack attribution.
2. **The Store trait itself has no actor concept.** The underlying `Store::update_issue(id, issue)` method takes no actor; the actor is only tracked in the `StoreWithEvents` wrapper and only in the in-memory `MutationPayload`. It is never persisted.
3. **Restrictions (pre-mutation policies) have no actor context.** `RestrictionContext` only carries `operation`, `payload`, and `store` -- there is no field for who is proposing the mutation. This blocks implementing permission checks (e.g., "only the issue creator can close it").
4. **Background workers act as the system** without any attribution model. When the `RunSpawnersWorker` creates a task for an issue, or the `GithubPollerWorker` updates a patch, the actor is `None`.

The goal is to make actor identity **required** on every state mutation so that:
- We can implement permission checks (restrictions that inspect the actor).
- We can implement full attribution (audit trail showing who changed what).
- Background workers have an explicit identity model.

## Goals

- Every mutation to AppState carries a **non-optional** actor identifier.
- Actor identity is persisted alongside each version of an object in the Store.
- The policy engine (both restrictions and automations) has access to the actor.
- Background workers and automations have a well-defined actor model.
- The change is incremental -- each step leaves the repo in a working state.

## Non-Goals

- Implementing specific permission policies (that is follow-up work once the plumbing is in place).
- Changing the authentication flow or token format.
- Adding a full RBAC/role system (future work that builds on this).
- Changing the API wire format (the actor is derived server-side from the auth token, not sent by the client).

## Current Architecture

### Actor Type
**`metis-server/src/domain/actors.rs:59-64`**

```rust
pub struct Actor {
    pub auth_token_hash: String,
    pub auth_token_salt: String,
    pub actor_id: ActorId,
}

pub enum ActorId {
    Username(Username),  // Human user: "u-alice"
    Task(TaskId),        // Agent job: "w-t-abc123"
}
```

### How Actor Flows Today

```
HTTP Request
  -> require_auth middleware (extracts Actor from Bearer token)
  -> Route handler (receives Extension<Actor>)
  -> AppState method (receives actor: Option<String> -- the actor.name())
  -> StoreWithEvents.*_with_actor(entity, actor: Option<String>)
     |-> Store trait method (NO actor param -- just entity + id)
     |-> EventBus emit (carries MutationPayload { ..., actor: Option<String> })
        |-> AutomationContext.actor() -> Option<&str>
```

**Key gaps:**
- `Store` trait methods: no actor parameter.
- `RestrictionContext`: no actor field.
- Background workers: pass `None` for actor.
- `MutationPayload.actor` and all `_with_actor` methods: `Option<String>`, not required.

### Background Workers (all in `metis-server/src/background/`)

| Worker | What it mutates | Current actor |
|--------|----------------|---------------|
| `ProcessPendingJobsWorker` | Updates task status (pending -> running) | `None` |
| `MonitorRunningJobsWorker` | Updates task status (running -> complete/failed) | `None` |
| `RunSpawnersWorker` | Creates tasks & issues for ready issues | `None` |
| `CleanupBranchesWorker` | Deletes git branches (no store mutation) | N/A |
| `GithubPollerWorker` | Updates patches from GitHub PR state | `None` |

### Policy Engine Automations (all in `metis-server/src/policy/automations/`)

| Automation | What it mutates | Current actor |
|-----------|----------------|---------------|
| `CascadeIssueStatusAutomation` | Updates dependent/child issue statuses | `None` (uses `app_state.upsert_issue(...)` with `None`) |
| `CloseMergeRequestIssuesAutomation` | Updates issue status on PR merge | `None` |
| `CreateMergeRequestIssueAutomation` | Creates merge-request issues for PRs | `None` |
| `KillTasksOnFailureAutomation` | Kills active tasks via job engine | No store mutation (engine call) |

### Policy Engine Restrictions (all in `metis-server/src/policy/restrictions/`)

| Restriction | What it checks | Needs actor? |
|------------|---------------|-------------|
| `IssueLifecycleRestriction` | Valid status transitions, children closed before parent | Not currently |
| `RequireCreatorRestriction` | Issue must have a non-empty `creator` field on creation | Could use actor |
| `TaskStateMachineRestriction` | Valid task status transitions | Not currently |
| `RunningJobValidationRestriction` | Job validity before running | Not currently |
| `DuplicateBranchRestriction` | No duplicate branch names on patches | Not currently |

## Proposed Approach

### New Type: `ActorRef`

Introduce a new type that distinguishes between different kinds of actors, replacing `Option<String>`:

```rust
/// Identifies who is performing a mutation.
///
/// Uses `ActorId` (the existing enum wrapping `Username` or `TaskId`) instead of
/// raw `String` for the `Authenticated` variant, per review feedback.
/// `on_behalf_of` and `triggered_by` also use `Option<ActorId>` so that
/// attribution chains are type-safe and consistently formatted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorRef {
    /// A human user or agent authenticated via Bearer token.
    Authenticated { actor_id: ActorId },
    /// A background worker acting on behalf of the system.
    /// `on_behalf_of` optionally attributes the action to a triggering actor.
    System {
        worker_name: String,
        on_behalf_of: Option<ActorId>,
    },
    /// A policy automation reacting to an event.
    /// `triggered_by` is the actor from the original event, if available.
    Automation {
        automation_name: String,
        triggered_by: Option<Box<ActorRef>>,
    },
}
```

> **Review feedback applied:** The reviewer requested using `ActorId` (the existing
> wrapper around `Username`/`TaskId`) instead of raw `String` within `ActorRef`.
> This ensures type safety and consistency with the existing `Actor.actor_id` field.
> The `Automation::triggered_by` field uses `Option<Box<ActorRef>>` to preserve the
> full actor chain (an automation may be triggered by another automation).

This replaces all `actor: Option<String>` parameters. `ActorRef` is always required (no `Option`), which the compiler enforces.

### Layer-by-Layer Changes

#### Layer 1: Store Trait

Add an `actor: &ActorRef` parameter to every mutation method in the `Store` trait:

```rust
// Before
async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<VersionNumber, StoreError>;

// After
async fn update_issue(&self, id: &IssueId, issue: Issue, actor: &ActorRef) -> Result<VersionNumber, StoreError>;
```

The Store implementations (MemoryStore, PostgresStore, PostgresStoreV2) will persist the actor alongside the version. For Postgres:
- Add an `actor JSONB` column to all versioned tables (v1 and v2).
- Migration: existing rows get `actor = NULL` (historical data predates this feature).

For the `Versioned<T>` wrapper:
```rust
pub struct Versioned<T> {
    pub item: T,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub actor: Option<ActorRef>,  // Option because historical versions won't have it
}
```

#### Layer 2: StoreWithEvents

The `_with_actor` methods on `StoreWithEvents` change signature from `actor: Option<String>` to `actor: ActorRef`. The duplicate non-actor `Store` trait impl on `StoreWithEvents` can be removed since actor is now always required.

#### Layer 3: AppState Methods

Every public mutation method on `AppState` takes `actor: ActorRef` instead of `actor: Option<String>`:

```rust
// Before
pub async fn upsert_issue(..., actor: Option<String>) -> ...
pub async fn create_job(..., actor: Option<String>) -> ...

// After
pub async fn upsert_issue(..., actor: ActorRef) -> ...
pub async fn create_job(..., actor: ActorRef) -> ...
```

#### Layer 4: Route Handlers

Route handlers construct `ActorRef::Authenticated { actor_id: actor.actor_id.clone() }` from the `Extension<Actor>` and pass it to AppState methods. This is a straightforward mechanical change.

#### Layer 5: Policy Engine

**RestrictionContext** gains an actor field:
```rust
pub struct RestrictionContext<'a> {
    pub operation: Operation,
    pub payload: &'a OperationPayload,
    pub store: &'a dyn ReadOnlyStore,
    pub actor: &'a ActorRef,  // NEW
}
```

All `PolicyEngine::check_*` shortcut methods gain an `actor: &ActorRef` parameter.

**AutomationContext** already has `actor()` via the event payload, but the type changes from `Option<&str>` to `&ActorRef` (always present).

**MutationPayload** changes from `actor: Option<String>` to `actor: ActorRef`.

#### Layer 6: Background Workers

Each worker constructs its own `ActorRef::System { worker_name, on_behalf_of }`:

| Worker | ActorRef |
|--------|----------|
| `ProcessPendingJobsWorker` | `System { worker_name: "process_pending_jobs", on_behalf_of: None }` |
| `MonitorRunningJobsWorker` | `System { worker_name: "monitor_running_jobs", on_behalf_of: None }` |
| `RunSpawnersWorker` | `System { worker_name: "run_spawners", on_behalf_of: Some(issue.creator) }` -- the issue creator triggered the spawn |
| `GithubPollerWorker` | `System { worker_name: "github_poller", on_behalf_of: None }` |

For automations that mutate state:
| Automation | ActorRef |
|-----------|----------|
| `CascadeIssueStatusAutomation` | `Automation { automation_name: "cascade_issue_status", triggered_by: event.actor }` |
| `CloseMergeRequestIssuesAutomation` | `Automation { automation_name: "close_merge_request_issues", triggered_by: event.actor }` |
| `CreateMergeRequestIssueAutomation` | `Automation { automation_name: "create_merge_request_issue", triggered_by: event.actor }` |

## Key Design Decisions

### 1. `ActorRef` vs extending the existing `Actor` type

We introduce a new `ActorRef` rather than reusing the existing `Actor` struct because:
- `Actor` is a persistence/auth entity (has token hashes, salts). It is too heavy to thread through every method.
- `ActorRef` is a lightweight identifier suitable for passing through call stacks and storing in version history.
- Background workers and automations don't have an `Actor` record in the store -- they need a different representation.

### 2. Non-optional actor (compiler-enforced)

Making `actor: ActorRef` non-optional (no `Option`) means the compiler catches every call site that forgets to provide one. This is the core value of the refactoring -- it makes "who did this?" an answered question everywhere, by construction.

### 3. Persisting actor in the Store

Storing the actor in each version row enables:
- Querying "who changed this issue?" from the version history.
- Building audit logs without relying on the ephemeral event bus.
- Future permission enforcement at the storage layer.

### 4. Automation triggered_by chain

When an automation fires in response to an event, it records the original event's actor as `triggered_by`. This preserves the causal chain: "user X closed issue A, which triggered cascade_issue_status to drop issue B."

## Risks and Open Questions

1. **Migration of historical data.** Existing version rows in Postgres will have `actor = NULL`. The `Versioned<T>` struct uses `Option<ActorRef>` to accommodate this. Should we backfill historical versions? Recommendation: no, leave them as `None` -- the cost is high and the value is low for historical data.

2. **Performance of JSONB actor column.** Each version row stores the actor as JSONB. For `System` and `Automation` variants this is a small object. Consider if a simple `TEXT` column (storing `ActorRef` as a formatted string) is more appropriate. The JSONB approach is more flexible for querying.

3. **Test fixtures.** Many tests pass `None` for actor. They will need to be updated to construct an `ActorRef`. A test helper like `ActorRef::test()` -> `ActorRef::System { worker_name: "test", on_behalf_of: None }` would reduce boilerplate.

4. **Event bus backward compatibility.** The SSE `/v1/events` endpoint streams events to clients. The `actor` field in events changes from `Option<String>` to `ActorRef`. This is a breaking change for SSE consumers -- they need to handle the new format. Since these are internal consumers, this is acceptable.

## Incremental Refactoring Plan

The refactoring is broken into 6 sequential PRs. Each leaves the repo compiling, with tests passing.

### PR 1: Introduce `ActorRef` type and add to `MutationPayload`
**Goal:** Define the new type and start using it in the event bus layer.
- Add `ActorRef` enum to `metis-server/src/domain/actors.rs` (or a new `metis-common` module if it needs to be shared with CLI).
- Add a `From<Option<String>>` impl for backward compatibility during migration.
- Change `MutationPayload::actor` from `Option<String>` to `ActorRef`.
- Update `MutationPayload::actor()` return type.
- Update `AutomationContext::actor()` return type.
- Update all call sites that construct `MutationPayload` (the `StoreWithEvents` methods).
- During this PR, the `_with_actor` methods still accept `Option<String>` and convert internally.
- **Key files:** `domain/actors.rs`, `app/event_bus.rs`, `policy/context.rs`
- **Tests:** Existing event-related tests pass; add unit tests for `ActorRef` serialization.

### PR 2: Thread `ActorRef` through `StoreWithEvents` and `AppState`
**Goal:** Replace `actor: Option<String>` with `actor: ActorRef` in the StoreWithEvents and AppState public APIs.
- Change all `_with_actor` method signatures on `StoreWithEvents` from `Option<String>` to `ActorRef`.
- Remove the duplicate `Store` trait impl on `StoreWithEvents` that delegates with `None` -- all callers must now provide an `ActorRef`.
- Change all public AppState mutation methods to take `ActorRef`.
- Update route handlers to construct `ActorRef::Authenticated { actor_id: actor.actor_id.clone() }`.
- Update background workers to construct `ActorRef::System { ... }`.
- Update automations to construct `ActorRef::Automation { ... }`.
- **Key files:** `app/event_bus.rs`, `app/issues.rs`, `app/patches.rs`, `app/documents.rs`, `app/jobs.rs`, `app/users.rs`, `routes/*.rs`, `background/*.rs`, `policy/automations/*.rs`
- **Tests:** All existing tests updated to pass `ActorRef`; add test helper `ActorRef::test()`.

### PR 3: Add `ActorRef` to `RestrictionContext`
**Goal:** Enable restrictions to make decisions based on who is performing the action.
- Add `actor: &'a ActorRef` field to `RestrictionContext`.
- Add `actor: &ActorRef` parameter to all `PolicyEngine::check_*` methods.
- Thread actor through from AppState methods that call policy checks.
- Existing restrictions ignore the new field (no behavior change).
- **Key files:** `policy/context.rs`, `policy/mod.rs`, `app/issues.rs`, `app/patches.rs`, `app/documents.rs`, `app/jobs.rs`
- **Tests:** All policy tests updated; add a test that restrictions can read the actor.

### PR 4: Add `actor` column to Store trait and Postgres schema
**Goal:** Persist the actor alongside each version in the database.
- Add `actor: &ActorRef` parameter to all `Store` trait mutation methods.
- Update `MemoryStore` to store `ActorRef` in each `Versioned<T>`.
- Add `actor: Option<ActorRef>` field to `Versioned<T>`.
- Write Postgres migration adding `actor JSONB` column to all versioned tables.
- Update `PostgresStore` and `PostgresStoreV2` to write and read the actor column.
- **Key files:** `store/mod.rs`, `store/memory_store.rs`, `store/postgres.rs`, `store/postgres_v2.rs`, `metis-common/src/versioning.rs`, new migration SQL
- **Tests:** Store tests verify actor is persisted and retrievable.

### PR 5: Expose actor in version history API
**Goal:** Make the actor visible in version history responses.
- Add `actor: Option<ActorRef>` to the version API response types in `metis-common`.
- Update activity log generation to include actor information.
- Update the `/v1/issues/:id/versions`, `/v1/patches/:id/versions`, `/v1/documents/:id/versions` endpoints.
- **Key files:** `metis-common/src/api/v1/issues.rs`, `metis-common/src/api/v1/patches.rs`, `metis-common/src/api/v1/documents.rs`, `metis-common/src/activity_log.rs`, `routes/issues.rs`, `routes/patches.rs`, `routes/documents.rs`
- **Tests:** API tests verify actor appears in version responses.

### PR 6: Clean up and remove backward-compatibility shims
**Goal:** Remove any remaining `Option<String>` actor code paths and the `From<Option<String>>` impl.
- Audit all remaining `Option<String>` actor references.
- Remove the `From<Option<String>> for ActorRef` conversion added in PR 1.
- Ensure no code path can produce an `ActorRef` from `None` without explicit intent.
- **Key files:** All files touched in previous PRs.
- **Tests:** Full test suite passes with no `Option<String>` actor paths remaining.
