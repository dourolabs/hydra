# Design: Issue State Space Expansion for Rejection and Failure Handling

**Author:** pm agent  
**Issue:** i-wimmrj  
**Status:** Draft  
**Date:** 2026-02-11

## Problem Statement

The current issue state machine has four statuses: `Open`, `InProgress`, `Closed`, `Dropped`. Two key workflows are unsupported:

1. **User-initiated replanning:** When a PM agent breaks a parent issue into child tasks and the user disagrees with part of the plan, there is no clean way to reject specific child issues and trigger replanning. The user can `Drop` an issue, but Dropped issues block downstream dependents and don't signal *why* the issue was rejected or that the parent should replan. The user must manually intervene to unstick the parent.

2. **Agent-initiated failure escalation:** When an SWE agent determines that an issue cannot be accomplished as described (e.g., the approach is wrong, requirements are unclear, or a prerequisite is missing), there is no way to signal this upward. The agent can only leave the issue `InProgress` and hope a human notices. The parent issue remains stuck waiting for all children to close.

## Current State Machine

### Statuses
| Status | Description |
|--------|-------------|
| `Open` | Initial state, waiting to be worked on |
| `InProgress` | Currently being worked on (or has children being worked on) |
| `Closed` | Completed successfully |
| `Dropped` | Abandoned; cascades to all children, kills active tasks |

### Readiness Rules
| Status | Ready When |
|--------|-----------|
| `Open` | All `BlockedOn` dependencies are `Closed` |
| `InProgress` | All children (via `ChildOf`) are `Closed` |
| `Closed` | Never |
| `Dropped` | Never |

### Key Behaviors
- **Dropping** cascades: all descendant issues are recursively marked `Dropped`, all their active tasks are killed.
- **Closing** is validated: all todos must be done, all children must be `Closed`, all blockers must be `Closed`.
- **Spawning** happens for any `Ready` issue assigned to an agent.
- **Task failures** are tracked at the task level (not the issue level); the spawner retries up to `max_tries` before giving up silently.
- **Dropped issues block** their downstream dependents (an Open issue blocked-on a Dropped issue is NOT ready).

### Current Gaps

1. `Dropped` is a terminal state that cascades destructively. There is no "soft rejection" — you either drop the entire subtree or leave it alone.
2. When a `Dropped` issue is a child, the parent becomes stuck: InProgress parents are only ready when ALL children are `Closed`, and Dropped children don't satisfy this.
3. There is no mechanism for a child issue to signal "I failed, parent should replan" — the parent just waits forever.
4. There is no field to capture *why* an issue was rejected or failed.

## Proposed Solution

### New Status: `Rejected`

Add a `Rejected` status to the `IssueStatus` enum. This status means: "This issue should not be done as specified; the parent needs to replan."

**Key properties of `Rejected`:**
- Does NOT cascade to children (unlike `Dropped`). Children of a rejected issue are dropped automatically, since the rejected issue will never be completed.
- Does NOT kill tasks on descendant issues (the cascade drop handles this).
- DOES make the parent issue `Ready` (unlike `Dropped`).
- An issue can be marked `Rejected` by either a user (replanning scenario) or by the agent working on it (failure escalation scenario).
- The `progress` field on the issue serves as the explanation of why it was rejected (agents/users should update progress before rejecting).

### New Status: `Failed`

Add a `Failed` status to the `IssueStatus` enum. This status means: "This issue was attempted but could not be completed as specified."

**Key properties of `Failed`:**
- Similar to `Rejected` in its effect on the parent (makes parent Ready).
- Does NOT cascade destructively to children.
- Children of a failed issue are dropped automatically.
- Can only be set by the agent working on the issue (or by an admin).
- The `progress` field captures what went wrong.
- Semantically distinct from `Rejected`: `Rejected` means "don't do this" (a planning/specification issue), while `Failed` means "tried and couldn't" (an execution issue).

**Alternative considered: single status.** We could use a single `Rejected` status for both cases. However, the distinction matters for the parent's replanning logic: if a child was rejected by a user, the parent knows the specification was wrong and needs a different approach. If a child failed, the parent knows the approach was attempted and needs debugging or a workaround. Keeping them separate gives the replanning agent more signal.

**Alternative considered: a `reason` field instead of using `progress`.** We could add a dedicated `rejection_reason` or `failure_reason` field. However, the `progress` field already exists and serves exactly this purpose — it's a free-text field that agents update as they work. Using it avoids schema bloat. The status itself (`Rejected` vs `Failed` vs `Dropped`) already conveys the category of the outcome.

### Updated Readiness Rules

| Status | Ready When |
|--------|-----------|
| `Open` | All `BlockedOn` dependencies are `Closed` |
| `InProgress` | All children are in a terminal state (`Closed`, `Rejected`, `Failed`, or `Dropped`) AND at least one child is `Rejected` or `Failed` (i.e., replanning is needed), OR all children are `Closed` (i.e., work is done) |
| `Closed` | Never |
| `Dropped` | Never |
| `Rejected` | Never |
| `Failed` | Never |

**Key change:** InProgress parents become ready when children reach any terminal state, not just `Closed`. This is the critical behavior that enables replanning.

**Nuance on readiness:** An InProgress issue is Ready when all children are terminal. The spawned agent then inspects the children's statuses to decide what to do:
- If all children are `Closed` → the work is done, close the parent (current behavior, unchanged).
- If any children are `Rejected` or `Failed` → the agent should replan (create new children, adjust approach, etc.).
- If any children are `Dropped` → this is an external intervention; the agent should assess and potentially replan.

This means we actually simplify the readiness rule: an InProgress parent is Ready when all children are in a terminal state (`Closed`, `Dropped`, `Rejected`, or `Failed`). The agent logic determines what to do based on the mix of child statuses. Currently, `Dropped` children make the parent stuck forever — this change also fixes that existing problem.

### Updated Readiness Rules (Revised)

| Status | Ready When |
|--------|-----------|
| `Open` | All `BlockedOn` dependencies are `Closed` |
| `InProgress` | All children are terminal (`Closed`, `Dropped`, `Rejected`, or `Failed`) |
| `Closed` | Never |
| `Dropped` | Never |
| `Rejected` | Never |
| `Failed` | Never |

### BlockedOn Behavior

For `BlockedOn` dependencies:
- `Rejected` and `Failed` blockers should **unblock** the dependent issue, same as `Closed`. The reasoning: if issue B is blocked-on issue A, and A is rejected, B should still proceed (possibly with a modified plan). The agent working on B can check A's status and adapt.
- This differs from `Dropped`, which continues to block. `Dropped` means "abandoned by external intervention" and the downstream issue may need manual attention. `Rejected`/`Failed` mean "this approach didn't work" and the system should continue processing.

**Alternative considered: Rejected/Failed blockers should also block.** This would be safer but would cause the same stuck-forever problem that Dropped currently has. The agent working on the dependent issue is better positioned to decide whether to proceed or not.

**Updated BlockedOn readiness rule:**

An `Open` issue is Ready when all its `BlockedOn` dependencies are in a "resolved" terminal state: `Closed`, `Rejected`, or `Failed`. `Dropped` dependencies continue to block (preserving current behavior for manual intervention cases).

### Cascade Behavior

| Action | Children | Active Tasks |
|--------|----------|-------------|
| `Dropped` | Recursively dropped (current behavior) | Killed (current behavior) |
| `Rejected` | Recursively dropped | Killed |
| `Failed` | Recursively dropped | Killed |

`Rejected` and `Failed` both cascade-drop their children and kill active tasks, just like `Dropped`. The difference is in what happens to the *parent*:
- `Dropped` child → parent stays stuck (current behavior; now parent becomes Ready)
- `Rejected` child → parent becomes Ready, agent replans
- `Failed` child → parent becomes Ready, agent replans

Wait — actually, with the revised readiness rule above, all three terminal-non-closed states make the parent Ready. This is the correct simplification. The distinction between Dropped, Rejected, and Failed is semantic signal for the replanning agent, not a difference in graph mechanics.

### Close Validation Updates

Current close validation requires all children to be `Closed`. This should be updated:
- All children must be in a terminal state (`Closed`, `Dropped`, `Rejected`, or `Failed`).
- At least... actually, no. If a parent is closing, it should mean the work is done. If children were Rejected or Failed, the parent shouldn't close unless new children were created to replace them, or the parent agent determined the work is complete despite the failures.

**Decision:** Keep current close validation — all children must be `Closed` for the parent to close. This forces the parent agent to deal with Rejected/Failed children (either by creating replacement tasks and closing them, or by dropping them explicitly and creating alternatives). This is the safest option and prevents premature closure.

For `BlockedOn` dependencies during close validation: require all blockers to be `Closed` (current behavior, unchanged). If a blocker was Rejected or Failed, the issue shouldn't have been workable in the first place... wait, we said Rejected/Failed unblock. So an issue that was unblocked by a Failed/Rejected blocker could have been worked on and might now want to close. The close validation should allow closing if all BlockedOn dependencies are in any terminal state (`Closed`, `Rejected`, `Failed`, or `Dropped`). This is consistent with the readiness rule.

**Updated close validation:**
- All todo items must be done (unchanged).
- All children must be `Closed` (unchanged — forces parent to handle rejected/failed children).
- All `BlockedOn` dependencies must be terminal (`Closed`, `Rejected`, `Failed`, or `Dropped`).

### CLI Changes

The `metis issues update` command already accepts `--status` with an `IssueStatus` value. Adding `Rejected` and `Failed` to the enum makes them available automatically.

No new CLI flags are needed. Users set `--progress "reason for rejection"` alongside `--status rejected`.

### Agent Prompt Implications

Agent prompts (PM and SWE) will need to be updated to understand the new statuses:

**SWE agents** should be instructed:
- If you determine an issue cannot be completed as specified, update the progress field with an explanation and set the status to `failed`.
- Do NOT silently leave an issue in `in-progress` if you believe it's impossible.

**PM agents** should be instructed:
- When spawned on an InProgress parent with Rejected or Failed children, inspect the children's statuses and progress fields to understand what went wrong.
- Create new replacement child issues as needed. You may drop the rejected/failed children (they're already terminal, but their subtrees were cascade-dropped).
- If the original plan was fundamentally wrong, create an entirely new set of children.

### SSE Events

No new event types are needed. Status changes to `Rejected` or `Failed` will trigger the existing `IssueUpdated` SSE event, which the dashboard already handles.

## Summary of Changes

### Data Model Changes
1. Add `Rejected` variant to `IssueStatus` enum (both `metis-common` and `metis-server` copies).
2. Add `Failed` variant to `IssueStatus` enum (both `metis-common` and `metis-server` copies).
3. Serde serialization: `rejected` and `failed` (kebab-case).

### Readiness Rule Changes
4. InProgress readiness: all children terminal (Closed OR Dropped OR Rejected OR Failed), not just all Closed.
5. Open/BlockedOn readiness: unblocked when blocker is Closed, Rejected, or Failed (not Dropped).

### Cascade Changes
6. Rejected issues cascade-drop children and kill active tasks (same as Dropped).
7. Failed issues cascade-drop children and kill active tasks (same as Dropped).

### Validation Changes
8. Close validation for BlockedOn: allow terminal states (Closed, Rejected, Failed, Dropped), not just Closed.
9. Close validation for children: keep requiring all Closed (forces parent to handle failures).

### CLI/Display Changes
10. Add `rejected` and `failed` to IssueStatus display/parsing.
11. Dashboard should display Rejected/Failed with appropriate styling (e.g., red/orange).

### Agent Prompt Changes
12. Update SWE agent instructions to use `failed` status.
13. Update PM agent instructions to handle replanning on Rejected/Failed children.

## Migration / Backwards Compatibility

- Existing issues are unaffected; Open/InProgress/Closed/Dropped continue to work identically.
- The `#[serde(other)]` variant (`Unknown`) in the client-side enum handles forward-compatibility for older CLI versions encountering new statuses.
- No database migration needed beyond what the store layer handles (JSONB storage stores status as a string).

## Open Questions

1. **Should `Dropped` also make the parent Ready?** Currently, Dropped children cause the parent to be stuck. The revised readiness rules above include Dropped as a terminal state that makes the parent Ready. This fixes an existing bug/limitation, but is a behavioral change. Recommend: yes, include this change.

2. **Should we add a `rejection_reason` field separate from `progress`?** Current proposal uses `progress` for both. A separate field would be cleaner for querying/filtering but adds schema complexity. Recommend: start with `progress`, add a dedicated field later if needed.

3. **Max retries interaction:** Currently, the spawner tracks retry attempts per issue. When an issue is Rejected or Failed by the agent, this counts as a "completed" task (not a retry). The spawner should not retry a Rejected/Failed issue — the parent should handle replanning. This should work naturally since Rejected/Failed issues are not Ready. Confirm: no spawner changes needed for the new terminal states.