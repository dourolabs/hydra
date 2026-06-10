# Issues and the graph

Issues are the unit of work. The system decides what can be worked on from a combination of each issue's **status** and the **graph edges** that connect issues together.

## Status

Status is data-driven per project. Each project declares an ordered list of `StatusDefinition` rows ([`hydra-common/src/api/v1/projects.rs`](../../hydra-common/src/api/v1/projects.rs)); each issue carries a `StatusKey` that resolves against its project's list at response time. The relevant behavior flags on `StatusDefinition` are:

- `unblocks_parents` — when true, a `child-of` parent stops counting this issue as still-open.
- `unblocks_dependents` — when true, a `blocked-on` dependent stops being blocked by this issue.
- `cascades_to_children` — when true, transitioning into this status drops every non-terminal child.

The seeded default project (`j-defaul`, defined in [`20260607000000_seed_default_project.sql`](../../hydra-server/migrations/20260607000000_seed_default_project.sql) and mirrored in [`hydra-server/src/domain/projects.rs`](../../hydra-server/src/domain/projects.rs)) ships these five statuses, which match the historical hardcoded enum and keep legacy issues working without per-row migration:

| key           | unblocks_parents | unblocks_dependents | cascades_to_children |
|---------------|------------------|---------------------|----------------------|
| `open`        | false            | false               | false                |
| `in-progress` | false            | false               | false                |
| `closed`      | true             | true                | false                |
| `dropped`     | true             | false               | true                 |
| `failed`      | true             | false               | true                 |

Anywhere code needs to ask "is this status terminal?" the answer is `unblocks_parents`; "can a dependent of this issue move forward?" is `unblocks_dependents`. Per-project statuses opt in to either flag independently.

## Graph edges

Two edges in [`hydra-server/src/store/mod.rs`](../../hydra-server/src/store/mod.rs) carry the readiness signal (the full `RelationshipType` enum has more variants — `HasPatch`, `HasDocument`, `RefersTo`, `Created` — but only these two participate in `Ready`):

- `child-of` — sub-task relationship; a parent's readiness depends on its descendants.
- `blocked-on` — hard prerequisite; the source can't progress until the target closes.

Edges live in the relationship table and are addressed at the storage layer via `RelationshipType::{ChildOf, BlockedOn}`. The domain layer (`is_issue_ready`, `Issue.dependencies`) mirrors these as `IssueDependencyType::{ChildOf, BlockedOn}`. They are decoupled from issue rows so traversal stays cheap.

## Inferred `Ready` predicate

`Ready` is not stored; it is derived on demand by [`AppState::is_issue_ready`](../../hydra-server/src/app/issues.rs). The rules:

- `unblocks_parents=true` (the legacy terminal lane: `closed | dropped | failed`) → **never ready**. Terminal states stay terminal.
- Otherwise, ready when every `blocked-on` target has `unblocks_dependents=true` (legacy: only `closed` releases the dependent — `dropped` and `failed` still block).
- A parent with children is ready when no issue in the full child subtree is ready. A parent with no children is trivially ready; a parent whose descendants are all stuck (terminal or blocked) becomes ready again so a new agent can re-plan.

The subtree walk uses a `visited` set for cycle protection; a cycle resolves to "not ready" rather than infinite recursion.

## Cascade rules

When an issue transitions to a terminal failure status, two automations fire (see [`automations-vs-background-workers.md`](./automations-vs-background-workers.md)):

- [`cascade_issue_status`](../../hydra-server/src/policy/automations/cascade_issue_status.rs) — on transition into a status whose definition sets `cascades_to_children=true` (default project: `dropped` and `failed`; configurable via `trigger_statuses`), walks descendants via BFS and sets every non-terminal child to `dropped`.
- [`kill_sessions_on_enter`](../../hydra-server/src/policy/automations/kill_sessions_on_enter.rs) — on any transition into a status whose `on_enter.kill_sessions = true`, **or** on issue deletion (soft-delete/archive), kills any `Created`/`Pending`/`Running` sessions attached to the issue and closes any non-`Closed` conversations spawned from it.

Note the asymmetry between the two terminal flags: a `failed` issue cascades down `child-of` (because `cascades_to_children=true`) but **not** down `blocked-on` (because `unblocks_dependents=false`). A `blocked-on` dependent of a `failed` issue stays open but is not ready — only a status with `unblocks_dependents=true` (legacy: `closed`) clears the edge. This is what enables parent re-planning — see below.

## Parent re-planning

If every direct or indirect child of an `in-progress` parent is stuck, the parent itself becomes ready and the next spawn cycle gives it an agent. That agent can inspect the failures and create replacement children to recover.

Example: A is `in-progress` with children B and C, where C is `blocked-on` B. B fails. C is still `open` but blocked by a status whose `unblocks_dependents` is false, so C is not ready. No child of A is ready, so A becomes ready. An agent for A diagnoses B's failure and creates a replacement.

## Parent ↔ child spawn mutex

To avoid two agents racing on the same goal:

- A child will not spawn while its parent has a pending or running session.
- A parent will not spawn while any of its children have a pending or running session.

These two halves are enforced in different places inside [`AgentQueue::spawn_for_issue`](../../hydra-server/src/policy/automations/agent_queue.rs):

- The **child-side** check is direct: `parent_has_running_task` walks the issue's `ChildOf` dependencies and short-circuits if any parent has a `Pending` or `Running` session.
- The **parent-side** guarantee is indirect, via the readiness rules above: an `in-progress` parent is only ready when no child subtree issue is ready, and a child with a live session is itself `open` and ready (or already `in-progress`), so the parent stays not-ready until the child settles.

`existing_issue_ids` (carried in `AgentTaskState`) is a separate guard against the same issue being spawned twice — not part of the parent/child mutex. Capacity, assignment, and per-issue retry budget checks live alongside these in the same function.
