# Issues and the graph

Issues are the unit of work. The system decides what can be worked on from a combination of each issue's **status** and the **graph edges** that connect issues together.

## Status

`IssueStatus` (defined in [`hydra-common/src/api/v1/issues.rs`](../../hydra-common/src/api/v1/issues.rs)):

| Status        | Meaning                                                                |
|---------------|------------------------------------------------------------------------|
| `Open`        | Not started.                                                           |
| `InProgress`  | Picked up by an agent or human; may still spawn follow-up work.        |
| `Closed`      | Successfully finished. Terminal.                                       |
| `Dropped`     | Explicitly abandoned. Terminal; cascades to children.                  |
| `Failed`      | Could not be completed. Terminal; **does not** cascade to children.    |

`IssueStatus::is_terminal()` covers `Closed | Dropped | Failed`.

## Graph edges

Two edges in [`hydra-server/src/store/mod.rs`](../../hydra-server/src/store/mod.rs) carry the readiness signal (the full `RelationshipType` enum has more variants — `HasPatch`, `HasDocument`, `RefersTo`, `Created` — but only these two participate in `Ready`):

- `child-of` — sub-task relationship; a parent's readiness depends on its descendants.
- `blocked-on` — hard prerequisite; the source can't progress until the target closes.

Edges live in the relationship table and are addressed at the storage layer via `RelationshipType::{ChildOf, BlockedOn}`. The domain layer (`is_issue_ready`, `Issue.dependencies`) mirrors these as `IssueDependencyType::{ChildOf, BlockedOn}`. They are decoupled from issue rows so traversal stays cheap.

## Inferred `Ready` predicate

`Ready` is not stored; it is derived on demand by [`AppState::is_issue_ready`](../../hydra-server/src/app/issues.rs). The rules:

- `Closed | Dropped | Failed` → **never ready**. Terminal states stay terminal.
- `Open` → ready when every `blocked-on` target is `Closed`. A blocker that is `Dropped` or `Failed` still blocks — only `Closed` releases the dependent.
- `InProgress` → ready when no issue in the full child subtree is ready. A parent with no children is trivially ready; a parent whose descendants are all stuck (terminal or blocked) becomes ready again so a new agent can re-plan.

The subtree walk uses a `visited` set for cycle protection; a cycle resolves to "not ready" rather than infinite recursion.

## Cascade rules

When an issue transitions to a terminal failure status, two automations fire (see [`automations-vs-background-workers.md`](./automations-vs-background-workers.md)):

- [`cascade_issue_status`](../../hydra-server/src/policy/automations/cascade_issue_status.rs) — on transition into `Dropped` or `Failed` (configurable via `trigger_statuses`), walks descendants via BFS and sets every non-terminal child to `Dropped`.
- [`kill_tasks_on_issue_failure`](../../hydra-server/src/policy/automations/kill_tasks_on_failure.rs) — on the same transitions, kills any `Created`/`Pending`/`Running` sessions attached to the issue.

Note the asymmetry: `Failed` cascades down `child-of` but **not** down `blocked-on`. A `blocked-on` dependent of a `Failed` issue stays `Open`, but is not `Ready` (only a `Closed` blocker clears the edge). This is what enables parent re-planning — see below.

## Parent re-planning

If every direct or indirect child of an `InProgress` parent is stuck, the parent itself becomes `Ready` and the next spawn cycle gives it an agent. That agent can inspect the failures and create replacement children to recover.

Example: A is `InProgress` with children B and C, where C is `blocked-on` B. B fails. C is still `Open` but blocked by a non-`Closed` issue, so C is not ready. No child of A is ready, so A becomes ready. An agent for A diagnoses B's failure and creates a replacement.

## Parent ↔ child spawn mutex

To avoid two agents racing on the same goal:

- A child will not spawn while its parent has a pending or running session.
- A parent will not spawn while any of its children have a pending or running session.

These two halves are enforced in different places inside [`AgentQueue::spawn_for_issue`](../../hydra-server/src/policy/automations/agent_queue.rs):

- The **child-side** check is direct: `parent_has_running_task` walks the issue's `ChildOf` dependencies and short-circuits if any parent has a `Pending` or `Running` session.
- The **parent-side** guarantee is indirect, via the readiness rules above: an `InProgress` parent is only `Ready` when no child subtree issue is ready, and a child with a live session is itself `Open` and ready (or already `InProgress`), so the parent stays not-ready until the child settles.

`existing_issue_ids` (carried in `AgentTaskState`) is a separate guard against the same issue being spawned twice — not part of the parent/child mutex. Capacity, assignment, and per-issue retry budget checks live alongside these in the same function.
