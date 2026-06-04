# Scenario: Per-Project Status Columnar Pipeline

**ID:** per-project-status-pipeline
**Category:** agent-coordination
**Priority:** P2
**Status:** queued — do not run until per-project status feature ([[i-ctfcvyru]]) has shipped. **Queue criterion:** all six implementation PRs of [[i-ctfcvyru]] ([[i-wnitrmch]], [[i-sxbbvtjq]], [[i-rlxcwaep]], [[i-nbcqsevh]], [[i-gulwytkr]], [[i-hpotgiuv]]) merged AND the tester agent prompt (`/agents/tester/prompt.md` in the doc store, `prompts/agents/tester.md` in the repo) updated to promote this scenario from P2 → P1.
**Prerequisites:**
- Server running (server-init scenario passed).
- Test-fixture repository `dourolabs/hydra-test-fixture` registered (add-github-repo scenario passed).
- Per-project-statuses feature ([[i-ctfcvyru]] / [[d-druoexk]]) has shipped on the server under test.
- One Project named `engineering-v2` created with the six statuses defined in the **Setup** section below.
- Three cloned agents `pm-v2`, `swe-v2`, `reviewer-v2` configured as active on `dourolabs/hydra-test-fixture` with the behavioral deltas defined in the **Setup** section below.

**Estimated duration:** ~30 minutes

## Description

Exercises the per-project status workflow designed in [[d-druoexk]] end-to-end through the dashboard. Covers both the **custom inbox/backlog/release pipeline** (design §4 "End-to-end use cases" — custom inbox/backlog/release pipeline) and the **same-issue review hand-off** (design §4 "End-to-end use cases" — same-issue review hand-off), exercising `apply_status_on_enter` automation (design §4 "Spawn dispatch and on_enter automation") and the unified readiness rule (design §4 "Dependencies, readiness, cascade") in one pass.

This is a **paper spec** authored ahead of implementation. It cannot be executed until the per-project-statuses feature has shipped; the first step is a skip-if pre-check that exits cleanly when the feature is absent so this scenario is safe to enable in the suite before all six implementation PRs land.

## Setup

The runner-side wiring needed to seed the configuration below (e.g. extending `tests/e2e/run.sh`, adding `tests/e2e/config/projects.yaml`, and authoring the cloned-agent prompt files in the doc store) is **out of scope** for this scenario file and is tracked separately. Anyone wiring it up should lift the spec below verbatim.

### Project `engineering-v2`

`default_status_key = "inbox"`. Statuses (in display order):

| key | unblocks_parents | unblocks_dependents | cascades_to_children | on_enter |
|---|---|---|---|---|
| `inbox` | false | false | false | None |
| `backlog` | false | false | false | `{ assign_to: Some(Principal::Agent { name: "pm-v2" }), attach_form: None }` |
| `pending` | false | false | false | None |
| `in-development` | false | false | false | `{ assign_to: Some(Principal::Agent { name: "swe-v2" }), attach_form: None }` |
| `in-review` | false | false | false | `{ assign_to: Some(Principal::Agent { name: "reviewer-v2" }), attach_form: Some("/forms/review.yaml") }` |
| `pending-release` | true | true | false | None |

Notes:
- `pending` exists as a holding state filed into directly by **Test bundle A** (Status UI surfaces); it has no automation.
- `pending-release` is terminal for dependency semantics (`unblocks_parents` and `unblocks_dependents` both true) but does **not** cascade to children (`cascades_to_children = false`), per design §4 "Dependencies, readiness, cascade".
- The status table format above mirrors the design's §4 "Default-project synthesis" table so the seeding spec stays aligned with how Hydra's built-in default project is described.

### Cloned agents

Each cloned agent is a clone of an existing base agent with the behavioral delta below. The actual prompt text (in the document store) is authored by the follow-up enablement task — this paper spec only declares the deltas.

- **`pm-v2`** — clone of `pm`. **Delta:** when filing child issues, sets each child's `project_id` to `engineering-v2` and lets the project's `default_status_key` (`inbox`) apply. `apply_status_on_enter` then routes child issues through the pipeline. Otherwise identical to `pm`.
- **`swe-v2`** — clone of `swe`. **Delta:** when done with a PR, instead of filing a child `review-request` issue, transitions the **same** issue from `in-development` → `in-review`. The `in-review.on_enter` rule reassigns to `reviewer-v2` automatically.
- **`reviewer-v2`** — clone of `reviewer`. **Delta:** picks up the **same** issue that SWE just transitioned (no child issue), runs the review against the attached `/forms/review.yaml`, and submits one of:
  - `request_changes` — form action emits `Effect::UpdateIssue { status: "in-development", set_feedback_from: Some("review_comment") }`.
  - `approve` — form action emits `Effect::UpdateIssue { status: "pending-release", set_feedback_from: None }`.

All three cloned agents are configured as active on `dourolabs/hydra-test-fixture`.

## Steps (via dashboard)

All interactions go through the dashboard at `http://localhost:8080` per the suite's convention. Steps use Playwright MCP for navigation, clicks, and assertions.

### Step 1 — Pre-check (skip-if)

1. Navigate to `http://localhost:8080/projects/engineering-v2`.
2. If the page returns a 404, redirects to a "not found" view, or otherwise indicates the per-project-statuses feature has not shipped (e.g. the route is unknown to the router), **mark this scenario `skipped` and exit**. A skip is a **pass**, not a failure.
3. If the Project editor page renders with the six statuses listed above, continue.

### Step 2 — Test bundle A: Status UI surfaces

For each of the six statuses (`inbox`, `backlog`, `pending`, `in-development`, `in-review`, `pending-release`), file one issue directly into that status:

1. Navigate to `/issues?project=engineering-v2`.
2. Click the "Create issue" button. In the new-issue form, set:
   - Title: `UI surface check — <status-key>`
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: `<status-key>` (selected from the project's status dropdown)
3. Submit. Note the issue id for later assertions.

For each issue filed above, assert:

- **List page.** Navigate to `/issues?project=engineering-v2`. The issue's status column renders the project's declared `label`, `icon`, and `color` for that status — **not** the legacy hardcoded mapping from `statusMapping.ts` (which is deleted by [[i-ctfcvyru]] PR 5 per design §4 "Frontend display"). Apply the status filter chip for that status; confirm the issue is included when its status is selected and excluded when a different status is selected.
- **Detail page.** Open the issue. The status badge renders with the same `label` / `icon` / `color` as on the list. Open the transition control; confirm the dropdown lists all six statuses in the project's defined order (`inbox`, `backlog`, `pending`, `in-development`, `in-review`, `pending-release`).
- **Related-items panels.** If the issue is referenced from any related-items panel (e.g. a parent's child list, a blocked-by panel on a dependent), the badge appearance there matches the list and detail views.
- **Filter dropdowns.** From any page that exposes a status filter dropdown (Issues list, Sessions filtered by issue status, etc.) when the project filter is set to `engineering-v2`, confirm all six statuses appear as filter options.

### Step 3 — Test bundle B: End-to-end columnar flow

1. From `/issues?project=engineering-v2`, click "Create issue". Fill in:
   - Title: `Make a small improvement to the hydra-test-fixture repo`
   - Description: `Make any small, low-risk improvement to the dourolabs/hydra-test-fixture repo at your discretion — for example, a typo fix, a minor wording polish, a tiny docs improvement, or an obviously-harmless cleanup. Use your judgment to pick the change. The goal is to submit a PR with a trivial change, not to make any specific edit.` (identical to the description used by `basic-issue-lifecycle.md`).
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: default (`inbox`).
2. Submit. Capture the issue id. This is the **single** id followed through every transition. **No child `review-request` issue may be created at any point during this bundle.**
3. From the issue detail page's transition control, manually transition `inbox` → `backlog`. Confirm `apply_status_on_enter` reassigns the issue to `pm-v2` and a `pm-v2` session spawns (visible on the Sessions page and on the issue detail page's activity log).
4. Wait for `pm-v2` to transition the issue to `in-development`. Confirm `swe-v2` is now the assignee (via `apply_status_on_enter`) and a `swe-v2` session spawns.
5. Wait for `swe-v2` to (a) produce at least one patch on `dourolabs/hydra-test-fixture` (visible on the patches page filtered by this issue id) and (b) transition the **same** issue id to `in-review`. Confirm `reviewer-v2` is the assignee and the review form (`/forms/review.yaml`) is attached.
6. Wait for `reviewer-v2` to submit the review form with `request_changes`. Confirm:
   - The issue status flips back to `in-development`.
   - The `issue.feedback` field is populated from the form's `review_comment` field (via `Effect::UpdateIssue { set_feedback_from: Some("review_comment") }` per design §4 "Spawn dispatch and on_enter automation").
   - `swe-v2` is reassigned via `apply_status_on_enter` and a new `swe-v2` session spawns.
7. Wait for `swe-v2` to address the feedback and transition the issue back to `in-review`.
8. Wait for `reviewer-v2` to submit `approve`. Confirm the issue status transitions to `pending-release`.

**Invariants (assert on the issue detail page and Sessions page):**

- The child-issue list on the test issue contains **no** child issue of type `review-request` (the same-issue review hand-off must not fall back to the child-issue pattern).
- The Sessions page shows at least one `swe-v2` session and at least one `reviewer-v2` session associated with this issue id, with `reviewer-v2` sessions spawning after `swe-v2` sessions (sequence preserved across the request-changes round-trip).
- The patches page shows at least one patch for this issue with a non-empty diff.

### Step 4 — Test bundle C: Dependency semantics

This bundle validates the unified readiness rule from design §4 "Dependencies, readiness, cascade" against `unblocks_parents` and `unblocks_dependents` flags.

**Parent / child setup (`unblocks_parents`):**

1. File a **parent** issue in `engineering-v2` directly into `in-development`:
   - Title: `Parent — unblocks_parents semantics check`
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: `in-development`
2. File **Child A** as `child-of: <parent-id>` in `engineering-v2`, status `pending-release` (`unblocks_parents=true`).
3. File **Child B** as `child-of: <parent-id>` in `engineering-v2`, status `in-review` (`unblocks_parents=false`).
4. On the parent's detail page, in the child-list panel, confirm:
   - Child A is shown as satisfying the parent's "child done" gate (its status's `unblocks_parents=true` flag visible / inferred from the panel).
   - Child B is shown as outstanding.
   - The parent is **not ready** (no agent spawns; readiness indicator absent on the parent).
5. Transition Child B to `pending-release` via its detail-page transition control. Reload the parent. Confirm:
   - Child B is now shown as satisfying the gate.
   - The parent is now **ready** (its `in-development` `on_enter` reassigns to `swe-v2`, which spawns).

**Dependent / blocker setup (`unblocks_dependents`):**

6. File a **blocker** issue in `engineering-v2` directly into `in-review` (`unblocks_dependents=false`).
7. File a **dependent** issue in `engineering-v2` directly into `in-development` with `blocked-on: <blocker-id>`.
8. On the dependent's detail page, in the blocked-by panel, confirm:
   - The blocker is shown as outstanding.
   - The dependent is **not ready** (no `swe-v2` spawn even though its own status would otherwise route to `swe-v2`).
9. Transition the blocker to `pending-release`. Reload the dependent. Confirm:
   - The blocker is now shown as satisfying the dependency.
   - The dependent is now **ready** and `swe-v2` is spawned via `apply_status_on_enter`.

## Expected Results

**Bundle A — Status UI surfaces:**
- Status badges on list, detail, related-items panels, and filter dropdowns all render the project's declared `label` / `icon` / `color` for each of the six statuses. None of the legacy hardcoded enum strings (`open`, `in-progress`, `closed`, `dropped`, `failed`) appears anywhere when the project filter is `engineering-v2`.
- The transition dropdown lists all six statuses in the project's defined order.
- The status filter on the issues list correctly includes / excludes issues per their status.

**Bundle B — End-to-end columnar flow:**
- A single issue id traverses `inbox` → `backlog` → `in-development` → `in-review` → `in-development` (after `request_changes`) → `in-review` → `pending-release`.
- At every transition into `backlog`, `in-development`, or `in-review`, `apply_status_on_enter` correctly reassigns to the configured agent (`pm-v2`, `swe-v2`, `reviewer-v2` respectively) and the assignee-driven spawn dispatcher spawns the corresponding session.
- On `request_changes`, `issue.feedback` is populated from the form's `review_comment` field and the next `swe-v2` session sees the feedback in its context.
- **No child `review-request` issue is created** at any point during this bundle. The Sessions page shows a coherent `swe-v2` → `reviewer-v2` → `swe-v2` → `reviewer-v2` sequence on the same issue id.
- At least one patch is produced on `dourolabs/hydra-test-fixture` and visible on the patches page.

**Bundle C — Dependency semantics:**
- A parent's "child done" gate counts a child in `pending-release` (`unblocks_parents=true`) as satisfied and a child in `in-review` (`unblocks_parents=false`) as outstanding. The parent only becomes ready once **every** direct child is in an `unblocks_parents=true` status.
- A dependent's "blocker done" gate counts a blocker in `pending-release` (`unblocks_dependents=true`) as satisfied and a blocker in `in-review` (`unblocks_dependents=false`) as outstanding. The dependent only becomes ready once **every** `blocked-on` dependency is in an `unblocks_dependents=true` status.
- Readiness changes are reflected in the dashboard's child-list and blocked-by panels and gate agent spawn correctly per the unified rule from design §4 "Dependencies, readiness, cascade".
