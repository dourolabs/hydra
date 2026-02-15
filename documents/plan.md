# Metis Repository Planning Notes

## Repository: dourolabs/metis
Metis is a server application with:
- `metis-server/` - Main server crate
  - `src/app/` - Application state and business logic
    - `app_state.rs` - Core AppState struct with repository, issue, and patch operations
    - `mod.rs` - Error types and module exports
  - `src/routes/` - HTTP route handlers
  - `src/domain/` - Domain models
  - `src/store/` - Data persistence layer

## Key Patterns
- Uses thiserror for error types
- Async/await throughout
- Error types often have multiple variants for different failure modes
- Routes map app errors to API errors with logging
- Rust allows multiple `impl` blocks for the same type across files within the same crate module

## Issue i-ybahlt: Test Structure Redesign (2026-02-15)
Task: Audit test redundancy and write design document for test restructuring.

### Status (updated 2026-02-15): Implementation tasks created, awaiting execution

### Design Document
Published at /designs/test-structure-redesign.md (document ID: d-yqhovt).
Review: i-borczi — approved by jayantk ("looks great, please proceed"), closed.

### Key Findings from Audit
- ~1000+ tests across 11 layers
- Route tests (metis-server/src/test/) are the biggest source of redundancy: they spin up real HTTP servers and re-test business logic already covered by policy and app layer tests
- Integration tests (metis/tests/harness_smoke_test.rs) include CRUD tests that duplicate route/store layer coverage
- Policy layer tests are well-scoped and serve as the authoritative tests for business rules (blocking, todos, children)
- App layer tests for issue readiness are correctly placed (unique logic)
- Store layer tests are correctly placed (pure persistence)

### Implementation Tasks (3 sequential PRs)
1. **i-xitpsf** — Simplify route tests (metis-server/src/test/) — remove ~20-30 tests, focus on HTTP behavior. No deps. Assigned to swe.
2. **i-nkakze** — Simplify integration tests (metis/tests/) — reduce harness_smoke_test.rs, simplify worker_issue_validation.rs. Blocked on i-xitpsf. Assigned to swe.
3. **i-mzrslc** — Remove duplicate policy tests from app layer (metis-server/src/app/) — remove 5-10 tests. Blocked on i-nkakze. Assigned to swe.

Expected net reduction: ~35-55 tests.

### Architecture Notes
- Route tests use `spawn_test_server()` which creates a real TCP listener and full Axum router
- Integration tests use `TestHarness` which wraps a TestServer + git remotes + GitHub mock
- App tests use `test_state()` which creates AppState with MemoryStore + MockJobEngine
- Policy tests use `RestrictionContext` with direct store access
- The `IssueLifecycleRestriction` in policy/restrictions/issue_lifecycle.rs is the canonical place for blocker/children/todo validation

## Issue i-fqmthm: AppState Restructuring (2026-02-14)
Task: Split app_state.rs (4182 lines) into logically coherent files

### Status (updated 2026-02-14 19:40 UTC)
Design approved (i-pkhfxv closed with "looks great"). Implementation tasks created (2026-02-14).
Tasks 1 and 2 merged. Task 3 (i-zqleuh) patch p-bpzlog submitted, review issue i-lpbfwl in-progress but no reviews posted yet.
Follow-up i-lkyhqg (move add_task() from agents.rs) also in-progress, patch p-gnrksh submitted (CI failure reported).
Once i-lpbfwl review completes and p-bpzlog merges, this issue can be closed.

### Design Document
Published at `/designs/app-state-restructuring.md`. Approved without revisions.

### Implementation Tasks (3 sequential PRs)

1. **i-unvkpi** — Extract shared test helpers to `app::test_helpers` module (no deps)
   - Move `sample_task()`, `task_for_issue()`, `state_with_default_model()`, `github_pull_request_response()`, `issue_with_status()`, `start_test_automation_runner()`, `TestAutomationRunner`, `poll_until()` to new `test_helpers.rs`
   - Prerequisite for all domain file extractions

2. **i-trtaqp** — Extract documents, repositories, agents, users, merge_queue modules (blocked on i-unvkpi)
   - 5 new files with `impl AppState` blocks for simpler domains
   - `create_actor_for_task()` becomes `pub(crate)` for cross-domain access from jobs

3. **i-zqleuh** — Extract issues, patches, jobs modules (blocked on i-trtaqp)
   - 3 new files for the complex, heavily-tested domains
   - After this, app_state.rs is ~100 lines (struct + constructor + accessors only)

### Key Findings from Analysis

**AppState struct (7 fields):**
- `config: Arc<AppConfig>` — global config
- `github_app: Option<Octocrab>` — GitHub integration client
- `service_state: Arc<ServiceState>` — merge queue + git cache
- `store: Arc<StoreWithEvents>` — data persistence with event emission (private)
- `job_engine: Arc<dyn JobEngine>` — Kubernetes job management
- `agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>` — agent queue configs
- `policy_engine: Arc<crate::policy::PolicyEngine>` — restrictions + automations

**Method count:** ~45 public methods + ~5 private methods across domains
**Error types in file:** 8 (CreateJobError, SetJobStatusError, UpsertPatchError, UpsertDocumentError, UpsertIssueError, UpdateTodoListError, AgentError, LoginError)
**Test lines:** ~2100 (lines 2016-4296)

### Cross-Domain Method Calls
- `create_actor_for_task()` in users.rs called from jobs.rs → needs `pub(crate)`
- All other cross-domain calls go through `self.store` or `self.policy_engine` which are struct fields

## Issue i-ocukfl: Build Cache Performance Regression (2026-02-14)
Task: Fix build cache performance regression after parallel upload/download changes

### Root Causes Identified
1. Parallel upload/download (commit 45739673) buffers entire files in memory → O(file_size) peak memory
2. metis-s3 server CPU-constrained at 500m with single replica
3. Per-phase timing instrumentation uses stderr (tracing::info!) but job logs only capture stdout

### Tasks Created
1. **i-vmkraf** — Fix streaming upload/download (repo: dourolabs/metis) → p-dkaael PR #1314 **MERGED**
2. **i-xptdjo** — Surface per-phase timings in job logs (repo: dourolabs/metis) → p-xshcah PR #1313 **MERGED**
3. **i-ircqhi** — Increase metis-s3 CPU 500m→2000m (repo: dourolabs/metis-cluster) → p-hhcfsc PR #24 **awaiting review**

### Status (2026-02-14 21:25 UTC)
2 of 3 patches merged. p-hhcfsc awaiting reviewer action on i-kuwybz. Once merged, this issue can be closed.

## Issue i-ugcnos: Policy Engine Design (2026-02-12)
Task: Design a policy engine to extract business logic from app_state.rs

### Status
Design document written and published at `/designs/policy-engine.md`. Review issue `i-xmhihp` assigned to jayantk.

### Implementation Tasks — Final State (as of 2026-02-13)

**Completed and merged (8 tasks):**
1. **i-kqwhbv** (closed) — Core traits, types, registry → merged
2. **i-ufnqxu** (closed) — Enrich event bus with mutation context → merged
3. **i-xcuenz** (closed) — Extract 6 restrictions → merged
4. **i-pyemvc** (closed) — Automation runner + extract 5 automations → merged
5. **i-pycdou** (closed) — Remove actor field from RestrictionContext → merged (follow-up)
6. **i-muwfar** (closed) — Refactor event discriminators to EventType enum → merged (follow-up)
7. **i-ljsbqm** (closed) — Add actor context to MutationPayload → merged (replacement for rejected i-bhjjvz part 1)
8. **i-spodpz** (closed) — Extract GitHub PR sync into automation → merged

**Remaining (1 task):**
- **i-cswfxw** (open) — TOML config integration, per-repo policy overrides, integration tests. No blockers, ready to start. This is the final task.

## Issue i-ryayev: Configurable Patch Workflow Automation (2026-02-15)
Task: Update merge request / patch tracking automation to support configurable workflows for patch creation, review, and merging.

### Status (updated 2026-02-15): Replanning after rejected approach

### Key Design Decision: Per-repo config inside the automation, not PolicyEngine
**Rejected approach (i-ymlcad):** Added `repos: HashMap<String, PolicyList>` to PolicyConfig and built separate ScopedEngine per repo.
**Reviewer feedback (jayantk):** Many events cannot be scoped to a repo. Per-repo config belongs inside individual automations, not the policy engine.
**Correct approach:** `patch_workflow` automation accepts a `repos` HashMap in its own TOML params for per-repo overrides.

### Task Breakdown (revised)

| # | Issue ID | Description | Status | Blocked on |
|---|----------|-------------|--------|------------|
| 1 | i-tiaeky | Add ReviewRequest issue type and Patch creator field | closed (merged) | — |
| 2 | i-xyikod | Implement patch_workflow automation with per-repo config in params | open | i-tiaeky |
| 3 | i-lkblsm | Sync ReviewRequest issue status with GitHub PR reviews | open | i-xyikod |
| 4 | i-gdchlg | Drop ReviewRequest issues when patch reaches terminal status | open | i-xyikod |

### Dropped Issues
- i-ymlcad (rejected) — per-repo PolicyEngine approach
- i-zlcbsf (dropped) — dependent on rejected approach
- i-nwcwkr (dropped) — dependent on dropped chain
- i-yxpqfi (dropped) — dependent on dropped chain