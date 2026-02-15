# Test Structure Redesign for Metis

## Problem Statement

The Metis codebase has ~1000+ tests spread across multiple layers, with significant redundancy. The same business logic (e.g., issue blocking validation, todo completion checks, child issue lifecycle) is tested at 3-4 different layers, making the test suite slow to run and expensive to maintain. When business rules change, multiple test files need updating for what is conceptually a single behavioral change.

## Goals

- Each architectural layer's tests should validate ONLY the behavior introduced at that layer.
- Eliminate redundant tests that re-verify logic already tested at a lower layer.
- Keep one set of integration tests that verify the layers compose correctly end-to-end.
- Make it clear where a new test should go when adding a feature.

## Non-Goals

- Changing the production architecture or layer boundaries.
- Rewriting working tests that are already at the correct layer.
- Achieving 100% code coverage metrics.

## Current Architecture Layers (Bottom to Top)

```
 ┌─────────────────────────────────────────────────────────┐
 │  Integration Tests  (metis/tests/)                      │
 │  Full server + CLI + git remotes + GitHub mock          │
 ├─────────────────────────────────────────────────────────┤
 │  Route Tests  (metis-server/src/test/)                  │
 │  Real HTTP server + in-memory store                     │
 ├─────────────────────────────────────────────────────────┤
 │  App Layer  (metis-server/src/app/)                     │
 │  AppState methods + MemoryStore + MockJobEngine         │
 ├─────────────────────────────────────────────────────────┤
 │  Policy Layer  (metis-server/src/policy/)               │
 │  Restrictions & Automations + MemoryStore               │
 ├─────────────────────────────────────────────────────────┤
 │  Background  (metis-server/src/background/)             │
 │  Spawner, scheduler, monitors                           │
 ├─────────────────────────────────────────────────────────┤
 │  Domain  (metis-server/src/domain/)                     │
 │  Types, parsing, status enums                           │
 ├─────────────────────────────────────────────────────────┤
 │  Store  (metis-server/src/store/)                       │
 │  MemoryStore, PostgreSQL persistence                    │
 ├─────────────────────────────────────────────────────────┤
 │  Common  (metis-common/src/)                            │
 │  Shared API types, IDs, wire formats                    │
 └─────────────────────────────────────────────────────────┘
```

## Current Test Inventory

### 1. Common Layer Tests (`metis-common/src/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `repo_name.rs` | ~5 | RepoName parsing/validation |
| `ids.rs` | ~5 | MetisId generation/formatting |
| `document_path.rs` | ~3 | Path normalization |
| `activity_log.rs` | ~3 | Log entry structure |
| `api/v1/issues.rs` | ~5 | Serialization round-trips |
| `api/v1/jobs.rs` | ~5 | Serialization round-trips |
| `api/v1/patches.rs` | ~3 | Serialization round-trips |
| `api/v1/documents.rs` | ~3 | Serialization round-trips |
| `api/v1/logs.rs` | ~2 | Log format |

**Assessment:** Well-scoped. These test types, parsing, and serialization. **No changes needed.**

### 2. Domain Layer Tests (`metis-server/src/domain/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `issues.rs` | ~9 | Status validation, dependency type handling, selector parsing, IssueGraphFilter |
| `jobs.rs` | ~3 | BundleSpec parsing, context resolution |
| `patches.rs` | ~5 | Status transitions, diff handling |
| `users.rs` | ~1 | Username handling |
| `task_status.rs` | ~1 | Status log parsing |
| `actors.rs` | ~4 | Actor creation/validation |

**Assessment:** Well-scoped. These test type-level logic (parsing, enum conversions). **No changes needed.**

### 3. Store Layer Tests (`metis-server/src/store/memory_store.rs`)

~84 tests covering:
- CRUD operations for all entities (issues, jobs, patches, documents, users, repos)
- Versioning and version history
- Soft deletion
- Duplicate rejection
- List operations with queries/pagination
- Dependency tracking (child-of, blocked-on)

**Assessment:** Well-scoped. These test pure persistence operations — data goes in, data comes out, versions increment. **No changes needed.**

### 4. Policy Layer Tests (`metis-server/src/policy/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `policy/tests.rs` | ~33 | Policy engine: restriction evaluation, automation execution, registry loading, config validation |
| `restrictions/issue_lifecycle.rs` | 7 | Blocking, open children, incomplete todos prevent closing |
| `restrictions/task_state_machine.rs` | 4 | Task state transitions |
| `restrictions/require_creator.rs` | 3 | Creator-only operations |
| `restrictions/duplicate_branch.rs` | 3 | Branch name conflicts |
| `restrictions/running_job_validation.rs` | 3 | Running job checks |
| `automations/cascade_issue_status.rs` | 6 | Status cascading to children |
| `automations/create_merge_request_issue.rs` | 7 | MR issue creation |
| `automations/close_merge_request_issues.rs` | 2 | MR closure on patch complete |
| `automations/kill_tasks_on_failure.rs` | 2 | Task termination on failure |
| `integrations/github_pr_sync.rs` | 5 | PR status sync |
| `integrations/github_pr_poller.rs` | 14 | PR polling logic |

**Assessment:** Well-scoped. Each restriction/automation is tested in isolation with a `RestrictionContext` or similar. **No changes needed.**

### 5. App Layer Tests (`metis-server/src/app/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `issues.rs` | 34 | Issue readiness (blocking, children, status), upsert validation, job association, deletion, status cascading |
| `jobs.rs` | 13 | Job lifecycle, bundle resolution, status tracking |
| `patches.rs` | 6 | Patch operations, status transitions |
| `documents.rs` | 2 | Document CRUD |
| `users.rs` | 2 | User operations |
| `event_bus.rs` | 12 | Event publishing, filtering, subscribers |
| `mod.rs` | 1 | App state initialization |

**Assessment: MIXED.** Some tests are well-scoped (event_bus, app initialization), but many duplicate logic tested elsewhere:

**Redundant tests in `app/issues.rs`:**
- `open_issue_ready_when_not_blocked` — tests readiness logic. This is app-layer logic not tested elsewhere, so it's **correctly placed**.
- `open_issue_not_ready_when_blocked_on_open_issue` — tests readiness. **Correctly placed** (readiness is app-layer logic, distinct from the restriction that prevents closing).
- `in_progress_issue_ready_after_children_closed` — tests readiness. **Correctly placed.**
- `dropped_issue_is_not_ready`, `dropped_blocker_keeps_issue_blocked`, `closed_issue_is_not_ready` — readiness logic. **Correctly placed.**
- `upsert_issue_rejects_missing_creator` — input validation. **Correctly placed.**
- `upsert_issue_rejects_missing_dependency` — input validation. **Correctly placed.**
- Tests related to status cascading and automation triggering — these overlap with policy layer tests. **Should be reviewed for redundancy.**

### 6. Background Layer Tests (`metis-server/src/background/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `spawner.rs` | 24 | Task spawning, agent queue management, rate limiting, issue readiness |
| `scheduler.rs` | 3 | Background task scheduling lifecycle |
| `run_spawners.rs` | 3 | Multi-spawner execution |
| `process_pending_jobs.rs` | 3 | Pending job processing |
| `monitor_running_jobs.rs` | 4 | Running job monitoring |
| `cleanup_branches.rs` | 21 | Branch cleanup logic |

**Assessment:** Mostly well-scoped. The spawner tests use `AppState` and `MemoryStore` to test spawning decisions — this is appropriate since spawning logic is this layer's responsibility. **No changes needed.**

### 7. Route Tests (`metis-server/src/test/`)

| Module | ~Count | Tests |
|--------|--------|-------|
| `issues.rs` | 21 | Issue CRUD, status changes, versioning, todos, dependencies, search, pagination |
| `jobs.rs` | 36 | Job CRUD, status transitions, image overrides, bundling, build cache config, logs |
| `patches.rs` | 22 | Patch CRUD, versioning, status, assets, diffs, GitHub integration |
| `documents.rs` | 13 | Document CRUD, versioning, paths, soft deletes |
| `users.rs` | 3 | User CRUD |
| `repositories.rs` | 5 | Repository registration and metadata |
| `agents.rs` | 6 | Agent list and metadata |
| `login.rs` | 6 | GitHub token login, session management |
| `auth.rs` | 2 | Authentication headers |
| `health.rs` | 1 | Health endpoint |
| `whoami.rs` | 2 | Identity endpoint |
| `github_token.rs` | 7 | GitHub token retrieval/persistence |
| `github_app.rs` | 1 | GitHub app client ID |
| `events.rs` | 6 | Event bus via HTTP |
| `merge_queues.rs` | 3 | Merge queue operations |

**Assessment: SIGNIFICANT REDUNDANCY.** Route tests spin up a real HTTP server (via `spawn_test_server()`) and make full HTTP requests. Many of these tests verify business logic that is already tested at the app or policy layer.

**Specific redundancies identified:**

| Route Test | Already Tested At | What's Actually New at Route Layer |
|-----------|-------------------|-----------------------------------|
| `update_issue_rejects_closing_when_blocked` | Policy: `rejects_closing_with_open_blockers` | Only the HTTP 400 status code mapping |
| `update_issue_rejects_closing_with_open_children` | Policy: `rejects_closing_with_open_children` | Only the HTTP 400 status code mapping |
| `update_issue_rejects_closing_with_open_todos` | Policy: `rejects_closing_with_incomplete_todos` | Only the HTTP 400 status code mapping |
| Issue CRUD tests (create, get, update, delete) | Store: `memory_store.rs` CRUD tests | HTTP serialization/deserialization, status codes |
| Job CRUD tests | Store layer + App layer | HTTP layer, image override HTTP flow |
| Patch CRUD tests | Store layer | HTTP layer |
| Document CRUD tests | Store layer | HTTP layer |

The route tests are doing ~80% business logic testing and ~20% HTTP layer testing. The business logic portion is redundant.

### 8. Integration Tests (`metis/tests/`)

| File | ~Count | Tests |
|------|--------|-------|
| `harness_smoke_test.rs` | 39 | Harness setup, issue CRUD via CLI, child issues, status updates, jobs |
| `harness_stepping_test.rs` | 11 | Task spawner, pending jobs, scheduling, GitHub sync |
| `harness_concurrency_test.rs` | 5 | Concurrent operations, ordering permutations |
| `harness_worker_test.rs` | 3 | Worker execution with git, patch creation |
| `cli_issue_flow.rs` | 1 | CLI issue creation with dependencies |
| `worker_cleanup_on_error.rs` | 1 | Worker error handling |
| `worker_issue_validation.rs` | 2 | Blocking and todo validation via worker commands |
| `worker_patch_flow.rs` | 1 | Patch creation via worker |
| `worker_patch_merge_flow.rs` | 1 | Full merge request workflow |
| `worker_patch_review_sync.rs` | 3 | GitHub PR sync scenarios |
| `metis_client_forward_compat.rs` | 1 | API forward compatibility |

**Assessment: MIXED.** Some tests are appropriate integration tests (worker flows, concurrency, forward compat). Others re-test things covered at lower layers.

**Redundant integration tests:**
- `harness_smoke_test.rs` tests like `user_handle_create_and_get_issue` — basic CRUD already covered by route tests and store tests. These should be minimal smoke checks, not thorough CRUD tests.
- `worker_issue_validation.rs` — re-tests blocking and todo validation. The policy layer already tests this; the integration test should only verify the CLI surfaces the error correctly.

**Well-placed integration tests:**
- `harness_worker_test.rs` — tests actual git operations and worker execution. Can only be tested at this layer.
- `harness_concurrency_test.rs` — tests concurrent access patterns. Appropriate for integration.
- `worker_patch_merge_flow.rs` — tests the full merge request lifecycle across layers. Appropriate.
- `metis_client_forward_compat.rs` — tests API contract stability. Appropriate.
- `harness_stepping_test.rs` — tests deterministic background task stepping. Appropriate.

### 9. Build Cache Tests (`metis-build-cache/`)

~8 integration tests in `tests/s3_integration.rs` + unit tests in `src/`.

**Assessment:** Well-scoped. **No changes needed.**

### 10. Config Tests (`metis-server/src/config/`)

~15 tests for configuration parsing.

**Assessment:** Well-scoped. **No changes needed.**

### 11. Job Engine Tests (`metis-server/src/job_engine/`)

~10 tests for Kubernetes job submission/monitoring.

**Assessment:** Well-scoped. **No changes needed.**

---

## Proposed Test Structure

### Principle: Each Layer Tests Only Its Own Behavior

| Layer | What to Test | What NOT to Test |
|-------|-------------|-----------------|
| **Common** | Types, serialization, parsing, ID formats | Nothing from other crates |
| **Domain** | Type construction, enum conversions, validation | No store, no app state |
| **Store** | CRUD, versioning, queries, pagination | No business rules, no HTTP |
| **Policy** | Each restriction/automation in isolation | No HTTP, no app orchestration |
| **App** | Readiness logic, orchestration, event bus | No HTTP, no policy rules (trust policy layer) |
| **Background** | Spawning decisions, scheduling, rate limiting | No HTTP, no policy rules |
| **Routes** | HTTP status codes, serialization, auth middleware, path parsing | No business logic validation |
| **Integration** | End-to-end workflows, worker+git, concurrency, forward compat | No re-testing of single-layer behaviors |

### Specific Changes

#### Route Tests (`metis-server/src/test/`) — LARGEST CHANGE

**Current problem:** Route tests re-test business logic by setting up scenarios and verifying outcomes through HTTP. For example, `update_issue_rejects_closing_when_blocked` creates a blocker, creates a blocked issue, tries to close it, and verifies the error — all through HTTP. This tests the policy restriction, the app layer, and the store, all at once.

**Proposed approach:** Route tests should focus on:
1. **Request/response format:** Does the endpoint accept the right JSON shape? Does it return the right JSON shape?
2. **HTTP status code mapping:** Does a policy violation become a 400? Does a not-found become a 404?
3. **Auth middleware:** Do unauthenticated requests get 401?
4. **Path/query parameter parsing:** Do filters, pagination, and path params work?
5. **Content negotiation:** SSE streams, multipart uploads, etc.

**Tests to remove or simplify:**

| Test | Action | Reason |
|------|--------|--------|
| `update_issue_rejects_closing_when_blocked` | **Simplify** — keep one test that verifies policy violations map to HTTP 400, remove the elaborate blocker setup | Blocking logic is tested in `policy/restrictions/issue_lifecycle.rs` |
| `update_issue_rejects_closing_with_open_children` | **Simplify** — same as above | Child validation tested in policy layer |
| `update_issue_rejects_closing_with_open_todos` | **Simplify** — same as above | Todo validation tested in policy layer |
| Issue CRUD tests that just create/read/update | **Keep but simplify** — verify HTTP status codes and response shapes, don't re-verify data integrity | Data integrity tested in store layer |
| Job CRUD business logic tests | **Simplify** — focus on HTTP-specific behavior (image override via HTTP headers, bundle resolution response format) | Business logic tested in app layer |

**Tests to keep as-is:**
| Test | Reason |
|------|--------|
| `auth.rs` tests | Authentication is route-layer behavior |
| `login.rs` tests | Login flow is route-layer behavior |
| `health.rs` test | HTTP health check |
| `whoami.rs` tests | Identity via HTTP |
| `events.rs` SSE tests | SSE streaming is route-layer behavior |
| Pagination/search filter tests | Query parameter handling is route-layer behavior |

#### Integration Tests (`metis/tests/`) — MODERATE CHANGE

**Tests to remove or simplify:**

| Test | Action | Reason |
|------|--------|--------|
| `harness_smoke_test.rs` basic CRUD tests | **Simplify** — reduce to 2-3 smoke tests that verify the harness works, not thorough CRUD | CRUD tested at route and store layers |
| `worker_issue_validation.rs` | **Simplify** — verify only that CLI returns an error when trying to close a blocked issue. Don't re-test all the blocking scenarios | Blocking logic tested in policy layer |

**Tests to keep as-is:**
| Test | Reason |
|------|--------|
| `harness_worker_test.rs` | Worker+git is only testable here |
| `harness_stepping_test.rs` | Background task coordination is only testable here |
| `harness_concurrency_test.rs` | Concurrency is only testable here |
| `worker_patch_flow.rs` | Worker+patch+git flow is only testable here |
| `worker_patch_merge_flow.rs` | Full merge lifecycle is only testable here |
| `worker_patch_review_sync.rs` | GitHub sync is only testable here |
| `metis_client_forward_compat.rs` | API contract is only testable here |
| `cli_issue_flow.rs` | CLI argument handling + dependency inheritance |

#### App Layer Tests (`metis-server/src/app/`) — MINOR CHANGE

**Tests to review:**
- Any tests that duplicate policy restriction behavior (e.g., if app tests verify that closing an issue with open children fails, and the policy layer already tests this via `IssueLifecycleRestriction`).
- The app layer's `upsert_issue` method calls into the policy engine. Tests should verify the *app's orchestration* (calling policy, applying result), not re-test the policy rules themselves.

**Tests to keep:**
- Issue readiness tests (`is_issue_ready`) — this logic lives in the app layer and is not tested elsewhere.
- Event bus tests — this logic lives in the app layer.
- Job bundle resolution — this logic lives in the app layer.

---

## Summary of Changes

| Layer | Current Tests | Action | Est. Tests After |
|-------|--------------|--------|-----------------|
| Common | ~35 | No change | ~35 |
| Domain | ~23 | No change | ~23 |
| Store | ~84 | No change | ~84 |
| Policy | ~91 | No change | ~91 |
| App | ~70 | Remove 5-10 tests that duplicate policy checks | ~60-65 |
| Background | ~58 | No change | ~58 |
| Routes | ~134 | Simplify ~20-30 tests to focus on HTTP behavior | ~110-120 |
| Integration | ~68 | Simplify ~15-20 smoke/validation tests | ~50-55 |
| Build Cache | ~8+ | No change | ~8+ |
| Config | ~15 | No change | ~15 |
| Job Engine | ~10 | No change | ~10 |

**Net reduction:** ~35-55 tests removed or consolidated. More importantly, the remaining tests will have clear ownership of what they verify, making them easier to maintain and faster to understand.

## Risks and Open Questions

1. **Coverage gaps:** When simplifying route tests, we need to ensure we don't lose coverage of HTTP-specific behavior (e.g., error response format). A good approach is to keep one "golden path" test per endpoint and one "error mapping" test, rather than testing every business rule through HTTP.

2. **Test infrastructure dependency:** The `TestHarness` in integration tests and `spawn_test_server` in route tests both spin up real HTTP servers. If we reduce route tests, we should verify the remaining integration tests still catch regressions in HTTP routing/middleware.

3. **App layer boundary:** The app layer orchestrates policy evaluation, store operations, and event emission. Some app tests inevitably touch the store and policy layers. The principle should be: app tests verify *orchestration* (e.g., "upsert_issue calls the policy engine and rejects if it fails"), not *business rules* (e.g., "closing with open children fails").

4. **PostgreSQL store tests:** The `memory_store.rs` has 84 tests. The PostgreSQL backends (`postgres.rs`, `postgres_v2.rs`) have fewer inline tests and may rely on integration tests for coverage. We should not remove integration tests that are the only path exercising PostgreSQL-specific behavior. (This is likely not an issue since integration tests use `MemoryStore`, but worth verifying.)

## Implementation Plan

The changes should be made in the following order:

1. **Route test simplification** — the largest and most impactful change. For each route test that re-tests business logic, either (a) reduce it to verify only the HTTP status code mapping, or (b) delete it if a simpler test already covers the HTTP behavior.

2. **Integration test cleanup** — simplify `harness_smoke_test.rs` CRUD tests and `worker_issue_validation.rs` to be true integration smoke tests rather than exhaustive business logic tests.

3. **App layer deduplication** — review app tests that test policy restriction behavior and remove the ones that are pure duplicates of policy tests.

Each step should leave the test suite green with `cargo test --workspace`.
