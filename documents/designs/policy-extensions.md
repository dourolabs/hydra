# Policy Extensions: Configurable Patch Review and Merge Workflows

## Problem Statement

Metis currently supports a single, partially-implemented workflow for patch review and merging: an agent creates a patch, a GitHub PR is created, a human reviews on GitHub, and the human merges. Several aspects of this workflow don't sync correctly (e.g., issue status from GitHub reviews, PR closure on issue failure), and there is no support for alternative workflows like agent-driven review or agent-driven merging.

The system needs to support two primary workflow modes, configurable per-repository:

1. **Human merge authority** (current workflow, with fixes): A human reviews and merges on GitHub. Metis syncs bidirectionally.
2. **Agent merge authority** (new): An agent or human reviews, and an agent or the merge queue handles merging. GitHub PRs are optional depending on reviewer type.

## Goals

- Fix existing bidirectional GitHub PR sync bugs (issue status from reviews, PR closure on issue drops).
- Support per-repository workflow configuration that selects the review and merge behavior.
- Allow configurable reviewer assignment (human or agent) per repository.
- Support agent-driven merging via the merge queue or direct merge, configurable per repo.
- Keep both workflows flexible enough that humans and agents can substitute for each other in any role.

## Non-Goals

- Building a full CI/CD system (we rely on GitHub Actions or external CI).
- Implementing a dedicated merge-conflict resolution agent (out of scope; we handle re-review requests).
- Multi-repo atomic changes.

## Current Architecture Summary

### Policy Engine

The policy engine has two tiers:
- **Restrictions** (pre-mutation): validate state transitions before persistence.
- **Automations** (post-mutation): react to events after persistence.

Automations are configured globally in `[policies]` TOML config. Policies can take parameters (e.g., `create_merge_request_issue = { assignee = "reviewer" }`). The `PolicyConfig` currently only has a global scope (`PolicyList`).

### Relevant Existing Automations

| Automation | Trigger | Behavior |
|---|---|---|
| `github_pr_sync` | PatchCreated/PatchUpdated (if `branch_name` set) | Creates/updates GitHub PR |
| `create_merge_request_issue` | PatchUpdated (ChangesRequested -> Open) | Creates MergeRequest issue for re-review |
| `close_merge_request_issues` | PatchUpdated (to Closed/Merged/ChangesRequested) | Closes or fails MergeRequest issues |
| `cascade_issue_status` | IssueUpdated (to terminal) | Drops child issues |
| `kill_tasks_on_issue_failure` | IssueUpdated (to terminal) | Kills running tasks |

### GitHub PR Poller (background worker)

Polls open patches with GitHub PR metadata every 60s. Syncs:
- PR status (open/closed/merged) -> patch status
- Reviews and comments -> patch reviews
- CI status -> patch CI metadata
- New non-approved reviews -> patch status to `ChangesRequested`

### Merge Queue

Exists as API endpoints (`GET/POST /merge-queues/{org}/{repo}/{branch}/patches`) and git-level cherry-pick logic. The CLI exposes `metis patches merge --repo <repo> --branch <branch> [--patch-id <id>]`. Currently used for enqueuing patches, but there is no automation that processes the queue.

### Identified Gaps in Current Implementation

1. **No automation creates a MergeRequest issue on initial patch creation** -- `create_merge_request_issue` only fires on ChangesRequested -> Open transitions, not on initial patch creation.
2. **No issue-to-PR closure sync** -- When a working issue transitions to dropped/failed/rejected, the associated PR is not closed.
3. **No per-repo policy configuration** -- Policies are global; different repos cannot have different workflows.
4. **No agent merge path** -- Once a patch is approved, there's no automation or CLI command to merge it onto the target branch.
5. **No merge queue processing** -- Patches can be enqueued but nothing processes the queue.
6. **`create_merge_request_issue` missing initial trigger** -- A MergeRequest issue should be created when a patch is first created (PatchCreated), not only on re-open.

## Proposed Approach

### 1. Per-Repository Policy Configuration

Extend `PolicyConfig` to support per-repo policy overrides. The global config provides defaults; per-repo configs override specific automations or their parameters.

**Config structure:**

```toml
# Global defaults
[policies]
restrictions = ["issue_lifecycle_validation", "task_state_machine", ...]
automations = ["cascade_issue_status", "kill_tasks_on_issue_failure", ...]

# Per-repo overrides
[policies.repos."dourolabs/metis"]
automations = [
  "cascade_issue_status",
  "kill_tasks_on_issue_failure",
  { name = "patch_workflow", params = { mode = "human_merge", reviewer = "jayantk" } },
]
```

**Changes to `PolicyConfig`:**

```rust
pub struct PolicyConfig {
    #[serde(flatten)]
    pub global: PolicyList,

    #[serde(default)]
    pub repos: HashMap<String, PolicyList>,
}
```

**Changes to `PolicyEngine`:**

The `PolicyEngine` needs to resolve which policy list applies for a given operation. For operations involving patches (which have `service_repo_name`), use the repo-specific policy list if one exists, falling back to the global list. The engine method signatures change to accept an optional `RepoName`:

```rust
impl PolicyEngine {
    pub async fn check_create_patch(&self, patch: &Patch, repo: Option<&RepoName>, store: &dyn ReadOnlyStore) -> Result<(), PolicyViolation>;
    pub async fn run_automations(&self, event: &ServerEvent, repo: Option<&RepoName>, ctx: &AutomationContext) -> Vec<AutomationError>;
}
```

Alternatively, the automation runner can resolve the repo from the event payload (patches carry `service_repo_name`, issues can be linked to patches via `patches` field).

### 2. Unified Patch Workflow Automation (`patch_workflow`)

Replace the three separate patch-related automations (`github_pr_sync`, `create_merge_request_issue`, `close_merge_request_issues`) with a single configurable `patch_workflow` automation that orchestrates the full patch lifecycle based on a `mode` parameter.

**Parameters:**

```toml
{ name = "patch_workflow", params = {
    mode = "human_merge",      # or "agent_merge"
    reviewer = "jayantk",      # reviewer assignee (human username or agent queue name)
    merge_strategy = "direct", # "direct" | "merge_queue" | "merge_agent"
    require_ci = true,         # whether to gate merge on CI success
    create_github_pr = true,   # whether to create a GitHub PR (auto-determined if not set)
}}
```

**Modes:**

**`human_merge` mode** (Workflow 1):
- On PatchCreated: Create GitHub PR + Create MergeRequest issue assigned to `reviewer`
- GitHub poller syncs reviews/status back to patch
- On new non-approved review from GitHub: Patch -> ChangesRequested, MergeRequest issue -> Failed
- On GitHub approval: MergeRequest issue -> Closed
- On GitHub merge: Patch -> Merged, working issue updated
- On working issue -> terminal: Close GitHub PR
- Agent keeps iterating until human is satisfied

**`agent_merge` mode** (Workflow 2):
- On PatchCreated: Create MergeRequest issue assigned to `reviewer`
- If `reviewer` is a human: Also create GitHub PR, enable GitHub poller sync
- If `reviewer` is an agent: No GitHub PR needed (agent reviews via patch directly)
- Reviewer (agent or human) can approve, request changes, or escalate to a human
- On patch approved: Trigger merge based on `merge_strategy`:
  - `direct`: Apply patch to target branch via git (server-side)
  - `merge_queue`: Enqueue patch to merge queue, create tracking issue
  - `merge_agent`: Create issue assigned to a dedicated merge agent

**Lifecycle state machine for both modes:**

```
PatchCreated
  |
  v
[Create MergeRequest issue] ---> [Create GitHub PR if needed]
  |
  v
Review cycle (may repeat):
  Reviewer comments/requests changes
    -> Patch status: ChangesRequested
    -> MergeRequest issue: Failed (agent re-works)
  Agent updates patch
    -> Patch status: Open
    -> New MergeRequest issue created
  Reviewer approves
    -> MergeRequest issue: Closed
    |
    v
  [Merge phase]
    human_merge: Human merges on GitHub -> synced to Patch: Merged
    agent_merge: Automation triggers merge based on merge_strategy
```

### 3. Issue-to-PR Closure Sync (New Automation: `close_pr_on_issue_failure`)

Add a new automation (or incorporate into `patch_workflow`) that fires when an issue transitions to a terminal state (Dropped, Failed, Rejected) and closes any associated GitHub PRs.

**Trigger:** IssueUpdated to terminal status
**Action:**
1. Find all patches linked to the issue (`issue.patches`)
2. For each patch with a GitHub PR that is still open: close the PR via GitHub API
3. Update patch status to Closed

This addresses the missing sync described in the issue: "if the agent's working issue transitions to dropped, failed, or rejected, the PR is automatically closed."

### 4. Initial MergeRequest Issue on Patch Creation

Fix `create_merge_request_issue` (or the new `patch_workflow`) to also fire on `PatchCreated` events, not just on ChangesRequested -> Open transitions. This ensures every new patch gets a review tracking issue.

### 5. Merge Execution

Add server-side merge capability that the `patch_workflow` automation (or CLI) can invoke:

**`metis patches merge` CLI changes:**

The existing CLI command (`metis patches merge --repo <repo> --branch <branch> --patch-id <id>`) currently only enqueues to the merge queue. Extend it with a `--strategy` flag:

```
metis patches merge --repo <repo> --branch <branch> --patch-id <id> [--strategy direct|queue|agent]
```

- `direct`: Server applies the patch to the target branch. Returns success/failure. On conflict, returns an error (agent must rebase and retry).
- `queue` (current behavior): Enqueues to the merge queue.
- `agent`: Creates a child issue assigned to a merge agent.

**Server-side direct merge:**

Add a new API endpoint or extend the existing merge queue endpoint:

```
POST /v1/patches/{patch_id}/merge
{
    "strategy": "direct",
    "target_branch": "main"
}
```

The server:
1. Verifies patch is approved (has at least one approved review or MergeRequest issue is closed)
2. Applies the patch to the target branch via git
3. Updates patch status to Merged
4. Returns result (success or conflict error)

### 6. Determining Reviewer Type

The system needs to know whether a reviewer is a human or an agent to decide whether to create a GitHub PR. The simplest approach: check if the reviewer name matches a configured agent queue name. If it does, it's an agent reviewer; otherwise, treat as human.

```rust
fn is_agent_reviewer(reviewer: &str, agent_queues: &[AgentQueueConfig]) -> bool {
    agent_queues.iter().any(|q| q.name == reviewer)
}
```

### 7. Re-review on Merge Conflicts

When a direct merge fails due to conflicts:
1. The merge API returns an error indicating the conflict
2. The agent rebases/updates the patch
3. If `require_re_review` is configured, the patch status is set to ChangesRequested, triggering a new review cycle
4. If not configured, the agent can retry the merge after rebasing

This is a configuration option on `patch_workflow`:

```toml
{ name = "patch_workflow", params = {
    ...
    require_re_review_on_conflict = false,  # default: false
}}
```

## Key Changes by File/Directory

| Area | Files | Change |
|---|---|---|
| Policy config | `metis-server/src/policy/config.rs` | Add `repos: HashMap<String, PolicyList>` to `PolicyConfig` |
| Policy engine | `metis-server/src/policy/mod.rs` | Support repo-scoped policy resolution |
| Policy registry | `metis-server/src/policy/registry.rs` | Register `patch_workflow` and `close_pr_on_issue_failure` |
| New automation | `metis-server/src/policy/automations/patch_workflow.rs` | Unified patch workflow automation |
| New automation | `metis-server/src/policy/automations/close_pr_on_issue_failure.rs` | Close PR when working issue fails |
| Merge API | `metis-server/src/routes/patches.rs` | Add `POST /patches/{id}/merge` endpoint |
| Merge logic | `metis-server/src/app/merge_queue.rs` | Add direct-merge logic |
| Repository model | `metis-common/src/api/v1/repositories.rs` | No change needed (per-repo config lives in policy config) |
| CLI | `metis/src/command/patches.rs` | Extend `merge` subcommand with `--strategy` |
| GitHub poller | `metis-server/src/policy/integrations/github_pr_poller.rs` | No structural changes; existing behavior serves both workflows |
| Existing automations | `create_merge_request_issue.rs`, `close_merge_request_issues.rs`, `github_pr_sync.rs` | Keep as-is for backward compatibility; `patch_workflow` replaces them when configured |

## Risks and Open Questions

1. **Backward compatibility**: Existing deployments use the three separate automations. The `patch_workflow` automation would replace them, but we should keep the old automations working for deployments that haven't migrated to per-repo config. A deployment can use either the old automations (global) or the new `patch_workflow` (per-repo), but not both for the same repo.

2. **Reviewer escalation**: The issue mentions that "the reviewer agent may also look at the PR and decide that it needs to call in a human to help." This could be implemented by the reviewer agent creating a child issue assigned to a human, or by reassigning the MergeRequest issue. The exact mechanism should be left to the agent's discretion rather than built into the automation framework.

3. **Merge queue processing**: The current merge queue stores patches but has no background worker to process them (e.g., attempt merges in order). If `merge_strategy = "merge_queue"` is used, we need either a background worker or a dedicated merge agent that watches the queue. Recommendation: use the `merge_agent` strategy for now (create an issue for a merge agent), and defer automated queue processing.

4. **CI gating**: The `require_ci` option needs to define what "CI passing" means -- currently `GithubCiStatus` tracks this via the poller, but for agent-only reviews without a GitHub PR, there's no CI signal. We may need a way to trigger and check CI independently of GitHub PRs, or require GitHub PRs even for agent-reviewed patches when CI is needed.

5. **Per-repo policy resolution complexity**: When an automation fires for a patch event, we can resolve the repo from `patch.service_repo_name`. But for issue events (e.g., `close_pr_on_issue_failure`), we need to look up the issue's linked patches to determine the repo. This adds a store lookup but is straightforward.

6. **Human flexibility in agent-merge mode**: The issue states "a human who is happy with the change may merge it themselves." This already works: if a GitHub PR exists, the human can merge it on GitHub, and the poller syncs the Merged status back. No additional work needed for this case.

## Proposed Task Breakdown

### Task 1: Fix initial MergeRequest issue creation on PatchCreated
Modify `create_merge_request_issue` to also fire on `PatchCreated` events, creating a MergeRequest tracking issue when a patch is first created (not just on re-open). This is a targeted fix to the existing automation.

### Task 2: Add `close_pr_on_issue_failure` automation
New automation that closes GitHub PRs when a working issue transitions to a terminal state. This fixes the missing bidirectional sync.

### Task 3: Add per-repo policy configuration
Extend `PolicyConfig` with per-repo policy lists. Update `PolicyEngine` to resolve repo-scoped policies. Update the automation runner to pass repo context.

### Task 4: Implement `patch_workflow` automation
New unified automation replacing the three existing patch automations. Supports `human_merge` and `agent_merge` modes with configurable reviewer and merge strategy. Includes logic for determining reviewer type (agent vs. human).

### Task 5: Implement server-side direct merge
Add `POST /patches/{id}/merge` endpoint and direct-merge logic. Extend CLI `metis patches merge` with `--strategy` flag.

### Task 6: Integration testing and documentation
End-to-end tests for both workflow modes. Update config documentation and AGENTS.md.
