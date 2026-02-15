# Policy Engine for Metis — Design Document

## Problem Statement

Metis server currently embeds all business logic rules directly in `app_state.rs`. These rules include:

- **Automations**: When a patch is created/updated, automatically create merge-request issues, cascade status changes to dependent issues, kill active tasks on failure, etc.
- **Restrictions/Validations**: Issue lifecycle validation (all blockers closed before closing, all children closed, all todos done), task state machine transitions, duplicate branch name checks, hidden path validation for documents, etc.
- **3rd party integrations**: GitHub PR sync, GitHub org allowlisting during login, CI status polling, etc.

This approach has several problems:
1. Changing any rule requires modifying server code and redeploying.
2. Different repos/teams cannot have different rules (e.g., different review paradigms).
3. Integrations are tightly coupled, making testing difficult and making it hard to add new integrations.
4. It's impossible to experiment with new workflows without changing core server logic.

## Goals

1. Extract business logic from `app_state.rs` into a configurable policy engine.
2. Support two kinds of policies: **automations** (reactive side effects) and **restrictions** (validation gates).
3. Make policies configurable per-repo and per-user, loadable at runtime from server config or CLI.
4. Leverage the existing event bus as the primary trigger mechanism for automations.
5. Separate 3rd party integrations into isolated policy modules for better testability.
6. Maintain backward compatibility — the default policy set reproduces current behavior exactly.

## Non-Goals

- A full-blown DSL or visual rule editor (future work).
- External policy engine service (policies run in-process).
- Hot-reloading policies without server restart in the initial implementation (future work; config reload could be added later).
- Policies that span multiple events or maintain their own persistent state (sagas/workflows).

## Proposed Approach

### Architecture Overview

```
                       ┌─────────────────────────────────┐
  API Request ──────►  │          AppState                │
                       │  ┌───────────────────────────┐   │
                       │  │  PolicyEngine              │   │
                       │  │  ┌───────────┐ ┌────────┐ │   │
                       │  │  │Restrictions│ │  Auto- │ │   │
                       │  │  │ (pre-write)│ │mations │ │   │
                       │  │  └─────┬─────┘ │(post-  │ │   │
                       │  │        │       │ write)  │ │   │
                       │  │        ▼       └────┬───┘ │   │
                       │  └───────────────────────────┘   │
                       │        │                  │       │
                       │        ▼                  ▼       │
                       │     Store            EventBus     │
                       │   (persist)         (broadcast)   │
                       └─────────────────────────────────┘
```

The policy engine has two evaluation points:

1. **Pre-write (Restrictions)**: Evaluated *before* a mutation is persisted. Can reject the operation with an error. These are synchronous validators.
2. **Post-write (Automations)**: Triggered *after* a mutation succeeds and an event is emitted. These perform side effects (create issues, update statuses, kill jobs, call external APIs).

### Core Types

```rust
/// A policy that validates a proposed mutation before it is persisted.
/// Returning Err rejects the mutation.
#[async_trait]
pub trait Restriction: Send + Sync {
    /// A unique name for this restriction (for config/logging).
    fn name(&self) -> &str;

    /// Evaluate the restriction. Return Ok(()) to allow, Err to reject.
    async fn evaluate(&self, ctx: &RestrictionContext) -> Result<(), PolicyViolation>;
}

/// A policy that reacts to a successfully persisted event.
#[async_trait]
pub trait Automation: Send + Sync {
    /// A unique name for this automation (for config/logging).
    fn name(&self) -> &str;

    /// Which events this automation subscribes to.
    fn event_filter(&self) -> EventFilter;

    /// Execute the automation's side effects.
    async fn execute(&self, ctx: &AutomationContext) -> Result<(), AutomationError>;
}

/// The proposed mutation that restrictions validate against.
pub struct RestrictionContext<'a> {
    pub operation: Operation,        // e.g., UpdateIssue, CreatePatch, etc.
    pub actor: &'a Actor,            // who is performing the operation
    pub repo: Option<&'a RepoName>,  // which repo, if applicable
    pub payload: &'a OperationPayload, // the proposed change (old + new state)
    pub store: &'a dyn Store,        // read-only access to current state
}

/// Context provided to automations when an event fires.
pub struct AutomationContext<'a> {
    pub event: &'a ServerEvent,      // the event that triggered this automation
    pub app_state: &'a AppState,     // full access to perform side effects
    pub store: &'a dyn Store,        // read access to current state
}

pub struct PolicyViolation {
    pub policy_name: String,
    pub message: String,
}
```

### PolicyEngine struct

```rust
pub struct PolicyEngine {
    restrictions: Vec<Box<dyn Restriction>>,
    automations: Vec<Box<dyn Automation>>,
}

impl PolicyEngine {
    /// Evaluate all restrictions for a proposed operation.
    /// Returns the first violation, if any.
    pub async fn check_restrictions(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        for restriction in &self.restrictions {
            restriction.evaluate(ctx).await?;
        }
        Ok(())
    }

    /// Run all automations that match the given event.
    /// Errors are logged but do not fail the original operation.
    pub async fn run_automations(&self, ctx: &AutomationContext<'_>) {
        for automation in &self.automations {
            if automation.event_filter().matches(&ctx.event) {
                if let Err(e) = automation.execute(ctx).await {
                    tracing::error!(
                        automation = automation.name(),
                        error = %e,
                        "automation failed"
                    );
                }
            }
        }
    }
}
```

### Event Filter

```rust
pub struct EventFilter {
    /// Which event types to match (empty = match all).
    pub event_types: Vec<EventType>,
    /// Optional repo filter.
    pub repo: Option<RepoName>,
}

pub enum EventType {
    IssueCreated,
    IssueUpdated,
    IssueDeleted,
    PatchCreated,
    PatchUpdated,
    PatchDeleted,
    JobCreated,
    JobUpdated,
    DocumentCreated,
    DocumentUpdated,
    DocumentDeleted,
}
```

### Integration with AppState

`AppState` gains a `PolicyEngine` field. The integration follows this pattern for each mutation method:

```rust
impl AppState {
    pub async fn upsert_issue(&self, request: UpsertIssueRequest) -> Result<Issue, UpsertIssueError> {
        // 1. Build the restriction context
        let ctx = RestrictionContext {
            operation: Operation::UpsertIssue,
            payload: &OperationPayload::Issue { /* old, new */ },
            // ...
        };

        // 2. Check restrictions (may reject)
        self.policy_engine.check_restrictions(&ctx).await
            .map_err(UpsertIssueError::PolicyViolation)?;

        // 3. Perform the actual mutation (persist to store)
        let result = self.do_upsert_issue(request).await?;

        // 4. Event is emitted by StoreWithEvents automatically
        // 5. Automations are triggered by the automation runner (see below)

        Ok(result)
    }
}
```

### Automation Runner

Instead of running automations inline (which would create complex call chains), we add a dedicated automation runner that subscribes to the event bus:

```rust
/// Spawned as a background task during server startup.
async fn automation_runner(policy_engine: Arc<PolicyEngine>, event_bus: EventBusSubscription, app_state: Arc<AppState>) {
    while let Ok(event) = event_bus.recv().await {
        let ctx = AutomationContext {
            event: &event,
            app_state: &app_state,
            store: app_state.store(),
        };
        policy_engine.run_automations(&ctx).await;
    }
}
```

This is a new background task (alongside the existing scheduler workers) that listens to the event bus and dispatches to automations. This approach:
- Decouples automations from the mutation path (no risk of blocking the API response).
- Leverages the existing event bus infrastructure.
- Keeps automation execution sequential per event (avoiding race conditions).
- Automation errors are logged but do not fail the original operation.

**Ordering guarantee**: Since the event bus is a broadcast channel, the automation runner processes events in order. Automations within a single event are also executed sequentially to avoid concurrency issues when multiple automations react to the same event.

### Configuration

Policies are configured in the server's TOML config file, under a new `[policies]` section:

```toml
# Global policies (apply to all repos unless overridden)
[policies]
# List of enabled restriction names. Comment out to disable.
restrictions = [
    "issue_lifecycle_validation",
    "task_state_machine",
    "duplicate_branch_name",
    "hidden_document_path",
    "require_progress_for_in_progress",  # new custom restriction
]

# List of enabled automation names.
automations = [
    "cascade_issue_status",
    "create_merge_request_issue",
    "close_merge_request_issues_on_patch_close",
    "kill_tasks_on_issue_failure",
    "github_pr_sync",
]

# Per-repo overrides
[policies.repos."dourolabs/metis"]
restrictions = [
    "issue_lifecycle_validation",
    "task_state_machine",
    "duplicate_branch_name",
    "hidden_document_path",
    # "require_progress_for_in_progress" is NOT enabled for this repo
]
automations = [
    "cascade_issue_status",
    "create_merge_request_issue",
    "close_merge_request_issues_on_patch_close",
    "kill_tasks_on_issue_failure",
    "github_pr_sync",
]
```

A **policy registry** maps names to implementations:

```rust
pub fn build_policy_engine(config: &PolicyConfig) -> PolicyEngine {
    let mut registry = PolicyRegistry::new();

    // Register all built-in policies
    registry.register_restriction("issue_lifecycle_validation", IssueLifecycleRestriction);
    registry.register_restriction("task_state_machine", TaskStateMachineRestriction);
    registry.register_restriction("duplicate_branch_name", DuplicateBranchRestriction);
    registry.register_restriction("hidden_document_path", HiddenDocumentPathRestriction);
    registry.register_restriction("require_progress_for_in_progress", RequireProgressRestriction);

    registry.register_automation("cascade_issue_status", CascadeIssueStatusAutomation);
    registry.register_automation("create_merge_request_issue", CreateMergeRequestIssueAutomation);
    registry.register_automation("close_merge_request_issues_on_patch_close", CloseMergeRequestAutomation);
    registry.register_automation("kill_tasks_on_issue_failure", KillTasksOnFailureAutomation);
    registry.register_automation("github_pr_sync", GitHubPrSyncAutomation);

    // Build engine from config (only enabled policies are included)
    registry.build(config)
}
```

### Extracting Current Business Logic

Here is the mapping of current embedded logic to named policies:

#### Restrictions (Pre-write Validators)

| Policy Name | Current Location | Description |
|---|---|---|
| `issue_lifecycle_validation` | `validate_issue_lifecycle()` lines 2462-2538 | When closing an issue: all blockers must be terminal, all todos done, all children Closed |
| `task_state_machine` | `set_job_status()` lines 2285-2360 | Valid task status transitions (Created→Pending→Running→Complete/Failed) |
| `duplicate_branch_name` | `upsert_patch()` lines 1619-1635 | Reject patches with branch names already used by open patches |
| `hidden_document_path` | `upsert_document()` lines 518-523 | Reject document paths with hidden segments (starting with `.`) |
| `running_job_validation` | `upsert_patch()` line 1611, `upsert_issue()` line 1985, `upsert_document()` line 578 | When `created_by` is a job ID, the job must be in Running status |
| `require_creator` | `upsert_issue()` lines 1861-1863, 2030-2031 | Issues must have a non-empty creator |

#### Automations (Post-write Side Effects)

| Policy Name | Current Location | Trigger | Description |
|---|---|---|---|
| `cascade_issue_status` | `upsert_issue()` lines 1897-1957 | Issue updated to Dropped/Rejected/Failed | Drop all children recursively; cascade to blocked-on dependents |
| `kill_tasks_on_issue_failure` | `upsert_issue()` lines 2069-2089 | Issue updated to Dropped/Rejected/Failed | Kill all active tasks for the issue and cascaded issues |
| `create_merge_request_issue` | `upsert_patch()` lines 1724-1726, `create_merge_request_review_issue()` lines 1733-1841 | Patch moves from ChangesRequested to Open | Create a new MergeRequest issue for the patch |
| `close_merge_request_issues` | `upsert_patch()` lines 1658-1722 | Patch moves to Closed/Merged/ChangesRequested | Close or fail all MergeRequest issues for this patch |
| `inherit_creator_from_parent` | `upsert_issue()` lines 2007-2028 | Issue created with empty creator | Inherit creator from parent issue (via ChildOf dependency) |

#### 3rd Party Integrations (as Automations)

| Policy Name | Current Location | Description |
|---|---|---|
| `github_pr_sync` | `sync_patch_with_github()` lines 679-764 | Create/update GitHub PR when patch is upserted with `sync_github_branch` |
| `github_pr_poller` | `poll_github_patches.rs` background worker | Poll GitHub for PR status, reviews, CI checks (stays as background worker, not event-driven) |
| `github_org_login_check` | `login()` lines 378-401 | Validate user belongs to allowed GitHub org during login |

**Note on the GitHub poller**: The GitHub PR poller is currently a background worker that polls on a timer. It should remain as a background worker (not an automation) because it's driven by external state changes in GitHub, not by internal events. However, it should be extracted into its own module under the policy/integration system for better isolation.

### Directory Structure

```
metis-server/src/
├── app/
│   ├── app_state.rs          # Slimmed down — delegates to policy engine
│   ├── mod.rs
│   └── event_bus.rs           # Unchanged
├── policy/
│   ├── mod.rs                 # PolicyEngine, Restriction, Automation traits
│   ├── registry.rs            # PolicyRegistry, build_policy_engine()
│   ├── context.rs             # RestrictionContext, AutomationContext, Operation types
│   ├── runner.rs              # automation_runner background task
│   ├── restrictions/
│   │   ├── mod.rs
│   │   ├── issue_lifecycle.rs
│   │   ├── task_state_machine.rs
│   │   ├── duplicate_branch.rs
│   │   ├── hidden_document_path.rs
│   │   ├── running_job_validation.rs
│   │   └── require_creator.rs
│   ├── automations/
│   │   ├── mod.rs
│   │   ├── cascade_issue_status.rs
│   │   ├── kill_tasks_on_failure.rs
│   │   ├── create_merge_request_issue.rs
│   │   ├── close_merge_request_issues.rs
│   │   └── inherit_creator.rs
│   └── integrations/
│       ├── mod.rs
│       ├── github_pr_sync.rs       # PR create/update automation
│       ├── github_pr_poller.rs     # Background worker (extracted from background/)
│       └── github_org_check.rs     # Login restriction
├── ...
```

### Per-Repo Policy Resolution

When evaluating policies, the engine resolves the effective policy set:

1. Start with the global policy list from `[policies]`.
2. If the operation is associated with a repo and `[policies.repos."<repo>"]` exists, use that repo's overrides instead.
3. If no repo-specific config exists, fall back to global.

This is a simple override model (not merge). If a repo specifies its own `restrictions` list, it completely replaces the global list for that repo.

### Error Handling

- **Restriction violations** produce a `PolicyViolation` error that is propagated to the caller as a 400 Bad Request with a descriptive message.
- **Automation failures** are logged with `tracing::error` but do not fail the original operation. This is important because the mutation has already been persisted.
- **Integration failures** (e.g., GitHub API errors) are similarly logged and do not block the core operation. This matches current behavior where GitHub sync errors are logged but don't prevent patch creation.

### Testing Strategy

Each policy is a standalone struct implementing a trait, making it trivially testable in isolation:

```rust
#[tokio::test]
async fn test_issue_lifecycle_restriction() {
    let store = MemoryStore::new();
    // set up test state...
    let restriction = IssueLifecycleRestriction;
    let ctx = RestrictionContext { /* ... */ };
    let result = restriction.evaluate(&ctx).await;
    assert!(result.is_err());
}
```

Integration tests verify that the full policy engine works end-to-end with the real `AppState` and `StoreWithEvents`.

## Risks and Open Questions

1. **Ordering of automations**: Some automations may depend on side effects of others (e.g., cascade status must happen before kill tasks). The current design runs automations sequentially in registration order, which should work if we register them in the right order. We could add explicit priority/ordering if needed.

2. **Re-entrant automations**: An automation that calls `app_state.upsert_issue()` will trigger another event, which will trigger more automations. This is intentional (cascade effects) but we need to guard against infinite loops. A depth counter or visited-set on the automation context could prevent this.

3. **Eventual consistency for automations**: Since automations run after the event is emitted (via the event bus), there's a brief window where the store is updated but automations haven't run yet. This is a change from the current behavior where side effects happen synchronously within the same method call. In practice this should be fine since the automation runner processes events immediately, but it's worth noting.

4. **GitHub poller as integration**: The GitHub PR poller doesn't fit neatly into the event-driven model since it's driven by external state. It should be extracted as a separate integration module but remain as a background worker. Its configuration could move under `[policies.integrations]`.

5. **Migration strategy**: The extraction should be done incrementally — one policy at a time — with each PR leaving the codebase in a working state. A "default policy set" should reproduce current behavior exactly, and tests should verify this.

6. **CLI configurability**: The issue mentions making policies configurable via CLI. This could be implemented as a `metis policies list` / `metis policies enable` / `metis policies disable` command that modifies the server config. This is future work beyond the initial extraction.

## Alternatives Considered

### 1. External DSL / Rule Engine (e.g., GoRules ZEN)
Considered using an external rules engine with a JSON-based decision model. Rejected because:
- The current rules are procedural (they call APIs, create issues, kill jobs) not declarative (they don't evaluate expressions against data).
- Adding a DSL increases complexity without proportional benefit for the current use case.
- Can be added later if the need for non-developer policy authoring arises.

### 2. Webhook / Plugin System
Considered implementing policies as external HTTP webhooks. Rejected because:
- Adds latency and reliability concerns for synchronous restrictions.
- The current integration needs (GitHub sync, issue cascade) require deep access to `AppState` and `Store`.
- Can be added later as an additional policy type (e.g., `WebhookAutomation`).

### 3. Lua/WASM Scripting
Considered embedding a scripting runtime for user-defined policies. Rejected for initial implementation because:
- Significantly more complex to implement safely.
- The initial need is configurability (enable/disable built-in rules), not extensibility (write new rules).
- Can be added later as a policy type.

## Key Changes Summary

| Area | Files Affected | Change |
|---|---|---|
| New policy module | `metis-server/src/policy/` (new) | Traits, registry, runner, all extracted policies |
| AppState simplification | `metis-server/src/app/app_state.rs` | Remove inline business logic, delegate to PolicyEngine |
| Event bus enhancement | `metis-server/src/app/event_bus.rs` | Add richer event payloads (old + new state) for automation context |
| Config expansion | `metis-server/src/config/mod.rs` | Add `[policies]` section with per-repo overrides |
| Background workers | `metis-server/src/background/` | Add automation_runner; extract GitHub poller to integrations |
| Error types | `metis-server/src/app/mod.rs` | Add PolicyViolation variant to mutation error types |
| Tests | `metis-server/src/test/` | Per-policy unit tests + integration tests for policy engine |
