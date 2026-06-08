# Scenario: Per-Project Status Columnar Pipeline

**ID:** per-project-status-pipeline
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:**
- Server running (server-init scenario passed).
- Test-fixture repository `dourolabs/hydra-test-fixture` registered (add-github-repo scenario passed).
- P0 scenarios (`basic-issue-lifecycle`, `dashboard-navigation`) and `pm-agent-breakdown` completed.
- Per-project-statuses feature ([[i-ctfcvyru]] / [[d-druoexk]]) has shipped on the server under test.
- Four-level prompt design ([[d-rzreslz]] / [[i-psuvqpqx]]) has shipped on the server under test (project- and status-level prompt slices concatenate into spawned sessions' `system_prompt`).
- One Project named `engineering-v2` created with the seven statuses defined in the **Setup** section below, and the five prompt documents (project prompt + the four non-terminal status prompts) pushed to the doc store.

**Estimated duration:** ~30 minutes

**Activated under [[i-bhaxdizc]]** after all six implementation PRs of [[i-ctfcvyru]] ([[i-wnitrmch]], [[i-sxbbvtjq]], [[i-rlxcwaep]], [[i-nbcqsevh]], [[i-gulwytkr]], [[i-hpotgiuv]]) merged. On a fresh single-player instance bootstrapped by `tests/e2e/run.sh`, the runner-side seeding wires up the `engineering-v2` Project and its prompts automatically. Step 1 below confirms the seeding landed; a missing project is a runner regression, not a skip.

## Description

Exercises the per-project status workflow designed in [[d-druoexk]] end-to-end through the dashboard, using the existing base `pm` / `swe` / `reviewer` agents. The behavioral deltas the scenario depends on — `pm` setting `project_id = engineering-v2` on child issues, `swe` doing same-issue review hand-off instead of filing a child `review-request`, and `reviewer` reviewing the same issue via the attached form — come from the **per-project + per-status prompt slices** introduced by the four-level prompt design ([[d-rzreslz]] / [[i-psuvqpqx]]), which concatenate onto each spawned session's `system_prompt`.

Covers both the **custom inbox/backlog/release pipeline** (design [[d-druoexk]] §4 "End-to-end use cases" — custom inbox/backlog/release pipeline) and the **same-issue review hand-off** (design [[d-druoexk]] §4 "End-to-end use cases" — same-issue review hand-off), exercising `apply_status_on_enter` automation (design [[d-druoexk]] §4 "Spawn dispatch and on_enter automation") and the unified readiness rule (design [[d-druoexk]] §4 "Dependencies, readiness, cascade") in one pass.

This scenario was authored ahead of implementation and lived under `scenarios/planned/` until [[i-bhaxdizc]] promoted it. Step 1 below is a hard pre-check that the runner-side `engineering-v2` seeding landed; if the project is missing, treat the run as failed (not skipped).

## Setup

The runner-side wiring needed to seed the configuration below is implemented by `tests/e2e/run.sh` (the project YAML fixture and prompt documents live under `tests/e2e/fixtures/projects/engineering-v2/`). Anyone re-implementing the seeding on a different runner should lift the spec below verbatim.

### Project `engineering-v2`

`default_status_key = "inbox"`. Statuses (in display order):

| key | unblocks_parents | unblocks_dependents | cascades_to_children | interactive | on_enter |
|---|---|---|---|---|---|
| `inbox` | false | false | false | false | None |
| `backlog` | false | false | false | false | `{ assign_to: Some(Principal::Agent { name: "pm" }), attach_form: None }` |
| `pending` | false | false | false | false | None |
| `in-development` | false | false | false | false | `{ assign_to: Some(Principal::Agent { name: "swe" }), attach_form: None }` |
| `pair-development` | false | false | false | true | `{ assign_to: Some(Principal::Agent { name: "swe" }), attach_form: None }` |
| `in-review` | false | false | false | false | `{ assign_to: Some(Principal::Agent { name: "reviewer" }), attach_form: Some("/forms/review.yaml") }` |
| `pending-release` | true | true | false | false | None |

Notes:
- `pending` exists as a holding state filed into directly by **Test bundle A** (Status UI surfaces); it has no automation.
- `pair-development` is the interactive variant of `in-development`: it carries `interactive: true` from [[d-ulhrefm]], so `AgentQueue` mints a `Conversation` (with `spawned_from = <issue_id>` and `greet_user: true`) instead of a headless `swe` session when a ready issue lands here. The `interactive-issue-mode` scenario ([[i-nuuyrbwl]]) drives this column; this scenario only asserts UI surfaces for it (Bundle A).
- `pending-release` is terminal for dependency semantics (`unblocks_parents` and `unblocks_dependents` both true) but does **not** cascade to children (`cascades_to_children = false`), per design [[d-druoexk]] §4 "Dependencies, readiness, cascade".
- The status table format above mirrors the design's §4 "Default-project synthesis" table so the seeding spec stays aligned with how Hydra's built-in default project is described.

### Project + status prompts

The four-level prompt resolver concatenates project- and status-level prompt slices onto each spawned session's `system_prompt`. The scenario depends on the following five documents being present in the doc store (the runner pushes them from the fixture files listed below):

| doc-store path | fixture file | encodes |
|---|---|---|
| `/projects/engineering-v2/prompt.md` | `tests/e2e/fixtures/projects/engineering-v2/prompt.md` | workflow narration — "this project routes work through `inbox → backlog → in-development → in-review → pending-release`, with `pending` as a holding state; reviews happen on the **same issue** via the form attached to `in-review`, not via child `review-request` issues; `apply_status_on_enter` routes assignment automatically; agents transition the issue by setting `--status <next>` on the same id." |
| `/projects/engineering-v2/statuses/backlog.md` | `tests/e2e/fixtures/projects/engineering-v2/statuses/backlog.md` | **`pm` delta** — when filing child issues, set their `project_id` to `engineering-v2` and let the project's `default_status_key` (`inbox`) apply; move this issue forward (to `pending` or `in-development`) when the breakdown is complete. |
| `/projects/engineering-v2/statuses/in-development.md` | `tests/e2e/fixtures/projects/engineering-v2/statuses/in-development.md` | **`swe` delta** — when the PR is ready, transition the **same** issue from `in-development` to `in-review` (do NOT file a child `review-request` issue — this project uses same-issue review hand-off); if a review brings the issue back to `in-development` with `feedback` populated, address the feedback and re-transition to `in-review`. |
| `/projects/engineering-v2/statuses/pair-development.md` | `tests/e2e/fixtures/projects/engineering-v2/statuses/pair-development.md` | **`swe` delta (interactive variant)** — same patch-and-handoff workflow as `in-development`, but the agent runs inside a spawned `Conversation` (`spawned_from = <issue_id>`, `greet_user: true`) instead of a headless session. Drives the `interactive-issue-mode` scenario ([[i-nuuyrbwl]]). |
| `/projects/engineering-v2/statuses/in-review.md` | `tests/e2e/fixtures/projects/engineering-v2/statuses/in-review.md` | **`reviewer` delta** — read the patch, decide a verdict, and submit the attached `/forms/review.yaml`: `request_changes` (transitions back to `in-development` and writes the form's `review_comment` field into `issue.feedback` via `Effect::UpdateIssue { set_feedback_from: Some("review_comment") }`) or `approve` (transitions to `pending-release`); do NOT file a child review-request — the form action drives both verdict and status transition. |

The project's `prompt_path` references `/projects/engineering-v2/prompt.md`; each non-terminal status's `prompt_path` (set in the project body) references its `/projects/engineering-v2/statuses/<key>.md` slice. Terminal statuses (`pending-release`, plus `inbox` and `pending` which carry no automation) have no `prompt_path` — the spawn dispatcher skips them anyway, so an empty slice is correct.

Prompts deliberately stay focused on the per-status delta. Cross-cutting agent behavior (how to use `hydra` CLI, `[[id]]` linking, etc.) lives in the system prompt; role-level guidance (how SWE thinks about patches, how reviewer formats verdicts) lives in the agent prompt. These status prompts only add what's unique to `engineering-v2` at each status.

## Steps (via dashboard)

All interactions go through the dashboard at `http://localhost:8080` per the suite's convention. Steps use Playwright MCP for navigation, clicks, and assertions.

### Step 1 — Pre-check

1. Navigate to `http://localhost:8080/projects/engineering-v2`.
2. Confirm the Project editor page renders with the seven statuses listed above. If it does not (404, redirect to a "not found" view, or the route is unknown to the router), treat this run as **failed** — the runner-side seeding in `tests/e2e/run.sh` should have wired up the project.

### Step 2 — Test bundle A: Status UI surfaces

For each of the seven statuses (`inbox`, `backlog`, `pending`, `in-development`, `pair-development`, `in-review`, `pending-release`), file one issue directly into that status:

1. Navigate to `/issues?project_key=engineering-v2`.
2. Click the "Create issue" button. In the new-issue form, set:
   - Title: `UI surface check — <status-key>`
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: `<status-key>` (selected from the project's status dropdown)
3. Submit. Note the issue id for later assertions.

For each issue filed above, assert:

- **List page.** Navigate to `/issues?project_key=engineering-v2`. The issue's status column renders the project's declared `label` and `color` for that status — **not** the legacy hardcoded mapping from `statusMapping.ts` (which is deleted by [[i-ctfcvyru]] PR 5 per design §4 "Frontend display"). Apply the status filter chip for that status; confirm the issue is included when its status is selected and excluded when a different status is selected.
- **Detail page.** Open the issue. The status badge renders with the same `label` / `color` as on the list. Open the transition control; confirm the dropdown lists all seven statuses in the project's defined order (`inbox`, `backlog`, `pending`, `in-development`, `pair-development`, `in-review`, `pending-release`). The `pair-development` row carries the project editor's "interactive" annotation chip alongside its label.
- **Related-items panels.** If the issue is referenced from any related-items panel (e.g. a parent's child list, a blocked-by panel on a dependent), the badge appearance there matches the list and detail views.
- **Filter dropdowns.** From any page that exposes a status filter dropdown (Issues list, Sessions filtered by issue status, etc.) when the project filter is set to `engineering-v2`, confirm all seven statuses appear as filter options.

### Step 3 — Test bundle B: End-to-end columnar flow

1. From `/issues?project_key=engineering-v2`, click "Create issue". Fill in:
   - Title: `Make a small improvement to the hydra-test-fixture repo`
   - Description: `Make any small, low-risk improvement to the dourolabs/hydra-test-fixture repo at your discretion — for example, a typo fix, a minor wording polish, a tiny docs improvement, or an obviously-harmless cleanup. Use your judgment to pick the change. The goal is to submit a PR with a trivial change, not to make any specific edit.` (identical to the description used by `basic-issue-lifecycle.md`).
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: default (`inbox`).
2. Submit. Capture the issue id. This is the **single** id followed through every transition. **No child `review-request` issue may be created at any point during this bundle.**
3. From the issue detail page's transition control, manually transition `inbox` → `backlog`. Confirm `apply_status_on_enter` reassigns the issue to `pm` and a `pm` session spawns (visible on the Sessions page and on the issue detail page's activity log).
4. Wait for `pm` to transition the issue to `in-development`. Confirm `swe` is now the assignee (via `apply_status_on_enter`) and a `swe` session spawns.
5. Wait for `swe` to (a) produce at least one patch on `dourolabs/hydra-test-fixture` (visible on the patches page filtered by this issue id) and (b) transition the **same** issue id to `in-review`. Confirm `reviewer` is the assignee and the review form (`/forms/review.yaml`) is attached.
6. Wait for `reviewer` to submit the review form with `request_changes`. Confirm:
   - The issue status flips back to `in-development`.
   - The `issue.feedback` field is populated from the form's `review_comment` field (via `Effect::UpdateIssue { set_feedback_from: Some("review_comment") }` per design [[d-druoexk]] §4 "Spawn dispatch and on_enter automation").
   - `swe` is reassigned via `apply_status_on_enter` and a new `swe` session spawns.
7. Wait for `swe` to address the feedback and transition the issue back to `in-review`.
8. Wait for `reviewer` to submit `approve`. Confirm the issue status transitions to `pending-release`.

**Invariants (assert on the issue detail page and Sessions page):**

- The child-issue list on the test issue contains **no** child issue of type `review-request` (the same-issue review hand-off must not fall back to the child-issue pattern).
- The Sessions page shows at least one `swe` session and at least one `reviewer` session associated with this issue id, with `reviewer` sessions spawning after `swe` sessions (sequence preserved across the request-changes round-trip).
- The patches page shows at least one patch for this issue with a non-empty diff.

### Step 4 — Test bundle C: Dependency semantics

This bundle validates the unified readiness rule from design [[d-druoexk]] §4 "Dependencies, readiness, cascade" against `unblocks_parents` and `unblocks_dependents` flags. To make the gate check deterministic, each sub-bundle wires up the full dep graph in a non-spawning status (`inbox`) first, then triggers the gate by transitioning the parent / dependent into a status whose `on_enter` would otherwise auto-spawn (`in-development`).

**Parent / child setup (`unblocks_parents`):**

1. File a **parent** issue in `engineering-v2` at `inbox` (no `on_enter`, no spawn):
   - Title: `Parent — unblocks_parents semantics check`
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: `inbox`
2. File **Child A** as `child-of: <parent-id>` in `engineering-v2`, status `pending-release` (`unblocks_parents=true`).
3. File **Child B** as `child-of: <parent-id>` in `engineering-v2`, status `in-review` (`unblocks_parents=false`). (A reviewer session may spawn for Child B itself — that's expected and unrelated to the parent gate.)
4. On the parent's detail page, in the child-list panel, confirm both Child A and Child B are listed as children.
5. Transition the parent from `inbox` to `in-development` via the detail-page transition control. Because the parent's `in-development` `on_enter` runs the assignee re-routing AND the gate check now sees both children, assert:
   - The parent's status is `in-development`.
   - Child A is shown as satisfying the parent's "child done" gate (its status's `unblocks_parents=true` flag visible / inferred from the panel).
   - Child B is shown as outstanding.
   - The parent is **not ready** — no `swe` session spawns (visible on the Sessions page filtered by this issue id).
6. Transition Child B from `in-review` to `pending-release` via its detail-page transition control. Reload the parent. Confirm:
   - Child B is now shown as satisfying the gate.
   - The parent is now **ready** and a `swe` session spawns within the dispatcher tick (via `apply_status_on_enter`).

**Dependent / blocker setup (`unblocks_dependents`):**

7. File a **blocker** issue in `engineering-v2` directly into `in-review` (`unblocks_dependents=false`). (A reviewer session may spawn for the blocker itself — expected.)
8. File a **dependent** issue in `engineering-v2` at `inbox` (no spawn) with `blocked-on: <blocker-id>`.
9. On the dependent's detail page, in the blocked-by panel, confirm the blocker is listed.
10. Transition the dependent from `inbox` to `in-development` via the detail-page transition control. Assert:
    - The dependent's status is `in-development`.
    - The blocker is shown as outstanding.
    - The dependent is **not ready** — no `swe` session spawns (even though its own status would otherwise route to `swe`).
11. Transition the blocker from `in-review` to `pending-release`. Reload the dependent. Confirm:
    - The blocker is now shown as satisfying the dependency.
    - The dependent is now **ready** and a `swe` session spawns via `apply_status_on_enter`.

## Expected Results

**Bundle A — Status UI surfaces:**
- Status badges on list, detail, related-items panels, and filter dropdowns all render the project's declared `label` / `color` for each of the seven statuses. None of the legacy hardcoded enum strings (`open`, `in-progress`, `closed`, `dropped`, `failed`) appears anywhere when the project filter is `engineering-v2`.
- The transition dropdown lists all seven statuses in the project's defined order; `pair-development` carries the "interactive" annotation chip.
- The status filter on the issues list correctly includes / excludes issues per their status.

**Bundle B — End-to-end columnar flow:**
- A single issue id traverses `inbox` → `backlog` → `in-development` → `in-review` → `in-development` (after `request_changes`) → `in-review` → `pending-release`.
- At every transition into `backlog`, `in-development`, or `in-review`, `apply_status_on_enter` correctly reassigns to the configured agent (`pm`, `swe`, `reviewer` respectively) and the assignee-driven spawn dispatcher spawns the corresponding session.
- On `request_changes`, `issue.feedback` is populated from the form's `review_comment` field and the next `swe` session sees the feedback in its context.
- **No child `review-request` issue is created** at any point during this bundle. The Sessions page shows a coherent `swe` → `reviewer` → `swe` → `reviewer` sequence on the same issue id.
- At least one patch is produced on `dourolabs/hydra-test-fixture` and visible on the patches page.

**Bundle C — Dependency semantics:**
- For both sub-bundles the dep graph is wired up while the parent / dependent sits in `inbox` (no `on_enter`, no spawn), so the subsequent transition into `in-development` is what triggers the readiness gate against the fully wired graph — there is no at-creation race against `on_enter` automation.
- When the parent transitions into `in-development`, the "child done" gate counts a child in `pending-release` (`unblocks_parents=true`) as satisfied and a child in `in-review` (`unblocks_parents=false`) as outstanding. The parent stays in `in-development` with no `swe` spawn until **every** direct child is in an `unblocks_parents=true` status; promoting the outstanding child into `pending-release` causes the next dispatcher tick to spawn `swe` for the parent.
- When the dependent transitions into `in-development`, the "blocker done" gate counts a blocker in `pending-release` (`unblocks_dependents=true`) as satisfied and a blocker in `in-review` (`unblocks_dependents=false`) as outstanding. The dependent stays in `in-development` with no `swe` spawn until **every** `blocked-on` dependency is in an `unblocks_dependents=true` status; promoting the outstanding blocker into `pending-release` causes the next dispatcher tick to spawn `swe` for the dependent.
- Readiness changes are reflected in the dashboard's child-list and blocked-by panels and gate agent spawn correctly per the unified rule from design [[d-druoexk]] §4 "Dependencies, readiness, cascade".
