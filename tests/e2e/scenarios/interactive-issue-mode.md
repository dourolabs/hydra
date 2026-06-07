# Scenario: Interactive Issue Mode

**ID:** interactive-issue-mode
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:**
- Server running (server-init scenario passed).
- Test-fixture repository `dourolabs/hydra-test-fixture` registered (add-github-repo scenario passed).
- Reviewer agent configured as active on the repository (same prereq as `reviewer-agent.md`); Bundle B's `pair-development → in-review` hand-off otherwise stalls waiting for the form submission.
- Per-project-statuses feature ([[i-ctfcvyru]] / [[d-druoexk]]) has shipped on the server under test.
- Four-level prompt design ([[d-rzreslz]] / [[i-psuvqpqx]]) has shipped on the server under test.
- Interactive-issue-mode feature ([[d-ulhrefm]]) has shipped — `StatusDefinition.interactive`, `Conversation.spawned_from`, `AgentQueue::build_task` dispatch, `SpawnConversationSessionsAutomation`, the `has_active_conversation` gate, and the `close_conversations_on_interactive_exit` automation are all wired on the instance under test (PRs 1–4 in [[i-xipntwnu]]).
- Frontend surfaces for interactive issues have shipped ([[i-lfwhatzq]]): issue-header "Open Conversation" affordance, Related-tab Conversations subsection, conversation-page "Originated from" link, project-editor `interactive` toggle, and the matching status-list annotation chip.
- One Project named `engineering-v2` created with the seven statuses defined in `per-project-status-pipeline.md`'s **Setup** section (including the new `pair-development` interactive status), and the five prompt documents pushed to the doc store. On a fresh single-player instance bootstrapped by `tests/e2e/run.sh`, this seeding is wired up automatically; Step 1 below confirms it landed.

**Estimated duration:** ~25 minutes

## Description

Exercises the interactive-issue-mode contract designed in [[d-ulhrefm]] end-to-end against the `engineering-v2` project's `pair-development` status (which carries `interactive: true`). The scenario walks one issue from creation, through an interactive `swe` conversation that produces a patch, into the standard same-issue review hand-off, and finally validates the `close_conversations_on_interactive_exit` direction by re-spawning a conversation and flipping its issue out into a non-interactive status.

Five assertions anchor the run, one per acceptance-criterion bullet on [[i-nuuyrbwl]]:

1. **Status-definition flag is honored at every UI surface** — the project editor and the issue's transition control both mark `pair-development` as interactive.
2. **AgentQueue mints a Conversation, not a headless Session** — a `Conversation` with `spawned_from = <issue_id>` appears in `hydra conversations list` (and the dashboard's chat list) within the dispatcher tick after the status transition.
3. **The conversation's first session is greeted autonomously** — `greet_user: true` is set on the spawn, and an assistant message appears in the chat before any human turn.
4. **Patches land through the standard reviewer chain** — `swe` running inside the conversation produces a patch on `dourolabs/hydra-test-fixture` and transitions the issue from `pair-development` to `in-review`, which then routes to `reviewer` and approves through to `pending-release` without any deviation from the headless reviewer chain.
5. **Closing the loop** — flipping an issue from `pair-development` into a non-interactive status closes the linked conversation (validated independently in Step 4 / Bundle C on a second, dedicated issue so Bundle B can run uninterrupted).

## Steps (via dashboard with CLI assertions)

All dashboard interactions go through `http://localhost:8080` per the suite's convention; CLI assertions use the bundled `hydra` binary against the same instance.

### Step 1 — Pre-check

1. Navigate to `http://localhost:8080/projects/engineering-v2`.
2. Confirm the Project editor page renders with the seven statuses defined in `per-project-status-pipeline.md`'s **Setup**. If it does not (404, redirect, or the project is missing), treat this run as **failed** — the runner-side seeding in `tests/e2e/run.sh` should have wired up the project.
3. Confirm `pair-development` is present in the status list with the project editor's "interactive" annotation chip rendered next to its label (PR 5 frontend, [[i-lfwhatzq]]). Open its row and confirm the `interactive` toggle is on.
4. From `/issues?project_key=engineering-v2`, open the "Create issue" form's status dropdown and confirm `pair-development` is listed with the same interactive chip rendered alongside its label.

### Step 2 — Test bundle A: Interactive spawn and greet-user

1. From `/issues?project_key=engineering-v2`, click "Create issue". Fill in:
   - Title: `Pair-mode improvement to the hydra-test-fixture repo`
   - Description: `Make any small, low-risk improvement to the dourolabs/hydra-test-fixture repo at your discretion — for example, a typo fix, a minor wording polish, a tiny docs improvement, or an obviously-harmless cleanup. Use your judgment to pick the change. The goal is to submit a PR with a trivial change, not to make any specific edit.` (identical to the description used by `per-project-status-pipeline.md` Bundle B and `basic-issue-lifecycle.md`).
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: default (`inbox`).
2. Submit. Capture the issue id as `${ISSUE_ID}`. This is the **single** id followed through every transition in this bundle. **No child `review-request` issue may be created at any point.**
3. From the issue detail page's transition control, transition `inbox` → `pair-development` directly (skipping the `backlog` PM step — this scenario tests the interactive path, not PM triage). Because the destination status definition carries `interactive: true`, `AgentQueue::build_task` must dispatch through the interactive branch, not `build_session_task`.
4. Within the next dispatcher tick (≤15s), assert all of:
   - **CLI:** `hydra conversations list --status active` returns at least one row, and the dashboard issue page (Related-tab Conversations subsection or the header's "Open Conversation" link) exposes the spawned conversation id. Capture it as `${CONV_ID}`. (The `--spawned-from` flag is not yet wired on the CLI per [[i-xipntwnu]] PR 2, and the list response uses `ConversationSummary` which does not include `spawned_from` — see `hydra-common/src/api/v1/conversations.rs:122`-`132` — so a JSONL `jq` filter on the list output won't work; use the dashboard or `hydra conversations get` to confirm `spawned_from` instead.)
   - **CLI:** `hydra conversations get ${CONV_ID}` returns `spawned_from == ${ISSUE_ID}` (the full `Conversation` shape carries the field) and a non-null `creator` matching the spawn-side actor (`agents/swe` per the project's `pair-development.on_enter`).
   - **Dashboard:** Navigate to `${ISSUE_ID}`'s detail page. The issue header renders an "Open Conversation" affordance (PR 5, [[i-lfwhatzq]]) linking to `/chat/${CONV_ID}`. The Related tab shows a Conversations subsection listing exactly one row: `${CONV_ID}` with status `Active`.
   - **Dashboard:** Click "Open Conversation". The conversation page header shows an "Originated from [[${ISSUE_ID}]]" link that resolves back to the issue detail.
   - **No headless `swe` session** appears on `/sessions?from=${ISSUE_ID}` for the `pair-development` entry — the interactive branch suppresses the headless spawn. (A headless `swe` session may exist later from the `request_changes` round-trip if Bundle B exercises it; this assertion is anchored to the pair-development entry only.)
5. Confirm `greet_user: true` and autonomous start:
   - **CLI:** `hydra sessions list --conversation ${CONV_ID}` returns at least one session row. Capture its `session_id` as `${SESSION_ID}`. The list response uses `SessionSummary` which exposes only `conversation_id` (see `hydra-common/src/api/v1/sessions.rs:680-714`), so use `hydra sessions get ${SESSION_ID}` for the `mode` and `env_vars` assertions below.
   - **CLI:** `hydra --output-format jsonl sessions get ${SESSION_ID} | jq '.mode'` returns `{ "type": "interactive", "conversation_id": "...", "greet_user": true }` (per [[d-ulhrefm]] §3 "Spawn behavior" — `SpawnConversationSessionsAutomation` mints `greet_user = spawned_from.is_some()`; the `SessionMode` enum is serialized with `#[serde(tag = "type", rename_all = "snake_case")]`).
   - **Dashboard:** On `/chat/${CONV_ID}`, the message list shows an assistant message **before** any user turn — i.e., the agent greeted autonomously. The greeting should reference the issue title or description (the agent picks up the prompt without a human prompt).
   - **CLI:** `hydra --output-format jsonl sessions get ${SESSION_ID} | jq -r '.env_vars.HYDRA_ISSUE_ID'` returns `${ISSUE_ID}` (per [[d-ulhrefm]] §3 — interactive sessions carry the same `HYDRA_ISSUE_ID` env var as headless ones, so SWE's standard CLI flow keeps working).

### Step 3 — Test bundle B: Patch lifecycle through reviewer chain

1. Continuing from Step 2 on the same `${ISSUE_ID}` / `${CONV_ID}`, wait for the in-conversation `swe` agent to:
   - Produce at least one patch on `dourolabs/hydra-test-fixture` (visible on `/patches?issue=${ISSUE_ID}` and via the issue's `patches` array: `hydra --output-format jsonl issues get ${ISSUE_ID} | jq -r '.issue.patches[]'` — `patches list` does not accept an `--issue` filter, so the issue body's `patches: Vec<PatchId>` field per `hydra-common/src/api/v1/issues.rs:280` is the CLI source of truth); and
   - Transition `${ISSUE_ID}` from `pair-development` to `in-review` via `hydra issues update ${ISSUE_ID} --status in-review` issued from inside the conversation session.
2. Assert that the `pair-development` → `in-review` transition fires two distinct automations:
   - `apply_status_on_enter` reassigns `${ISSUE_ID}` to `reviewer` and attaches `/forms/review.yaml` (the standard `engineering-v2` `in-review` rule).
   - `close_conversations_on_interactive_exit` closes the conversation `${CONV_ID}` because the new status (`in-review`) is non-interactive. Confirm via `hydra conversations get ${CONV_ID}` that the status is now `Closed`. On the issue detail page, the "Open Conversation" affordance disappears from the header; the Related-tab Conversations subsection still lists `${CONV_ID}` (now with status `Closed` per [[i-lfwhatzq]]'s "all statuses" query) so the audit trail is preserved.
3. Wait for `reviewer` to submit the review form with `approve`. Confirm `${ISSUE_ID}` transitions to `pending-release` (the same form's `approve` action used by `per-project-status-pipeline.md` Bundle B).
4. Invariants:
   - The child-issue list on `${ISSUE_ID}` contains **no** child issue of type `review-request` — interactive mode preserves same-issue review hand-off.
   - `hydra --output-format jsonl issues get ${ISSUE_ID} | jq -r '.issue.patches[]'` lists at least one patch id; `hydra patches get <patch-id>` for that id shows a non-empty diff against `dourolabs/hydra-test-fixture`.
   - `hydra sessions list --from ${ISSUE_ID}` shows two rows (the `SessionSummary` shape exposes `conversation_id` only; per-row `mode.type` / `mode.greet_user` must be re-read with `hydra sessions get <session-id>`): the in-conversation `swe` session has `conversation_id == ${CONV_ID}` and `mode == { "type": "interactive", "greet_user": true, ... }`; the `reviewer` session spawned after the `in-review` transition has `conversation_id == null` and `mode == { "type": "headless" }`. The two-mode mix on a single issue is the uniformity guarantee from [[d-ulhrefm]] §1 — patch and review semantics are mode-agnostic.

### Step 4 — Test bundle C: Close-on-exit via status flip-out

This bundle independently validates the `close_conversations_on_interactive_exit` automation against the manual flip-out direction (the user moves an interactive issue back to a non-interactive status mid-flight, instead of the agent transitioning forward to `in-review`).

1. From `/issues?project_key=engineering-v2`, file a second issue:
   - Title: `Pair-mode close-on-exit smoke`
   - Description: `Test fixture for the interactive-issue-mode close-on-exit assertion. Do not submit a patch — this issue exists to validate the conversation-close direction only.`
   - Repository: `dourolabs/hydra-test-fixture`
   - Project: `engineering-v2`
   - Status: default (`inbox`).
2. Capture the new issue id as `${ISSUE_ID_2}`. Transition `inbox` → `pair-development`.
3. Within the dispatcher tick, capture the spawned conversation id as `${CONV_ID_2}` (read it off the issue header's "Open Conversation" link or the Related-tab Conversations subsection, as in Step 2.4 — `ConversationSummary` doesn't expose `spawned_from`, so the dashboard is the practical source). Confirm via `hydra conversations get ${CONV_ID_2}` that `spawned_from == ${ISSUE_ID_2}` and the status is `Active`.
4. Atomically flip `${ISSUE_ID_2}` out of `pair-development` and clear its assignee in a single CLI update:

       hydra issues update ${ISSUE_ID_2} --status pending --clear-assignee

   - **Why:** `pair-development.on_enter.assign_to: swe` carries over to `pending` (which has no `on_enter`), so the AgentQueue would otherwise dispatch a headless `swe` session in `pending` (per [[d-ulhrefm]] §5 "Lifecycle"). Clearing the assignee in the same atomic update prevents that confounder.

   Within the next event-bus tick:
   - `hydra conversations get ${CONV_ID_2}` returns `status == Closed`.
   - The dashboard issue header no longer renders the "Open Conversation" affordance; the Related-tab Conversations subsection still lists `${CONV_ID_2}` with status `Closed`.
   - `hydra issues get ${ISSUE_ID_2}` shows `assignee` is either `null` OR the configured `assignment_agent` (default `agents/pm`). Both values are acceptable here — the determinative check is the no-headless-`swe`-spawn assertion in the next bullet.
     - **Why:** `SpawnSessionsAutomation` (`hydra-server/src/policy/automations/spawn_sessions.rs:144-186`) auto-assigns any unassigned issue to the configured `assignment_agent` on every event tick, racing the atomic `--clear-assignee` update by sub-milliseconds. The `null → pm` reassignment is by-design, so observing `agents/pm` on read does not mean `--clear-assignee` failed — the assignee transitioned through `null` (which is what gates the no-`swe`-spawn check) before the auto-assign reattached `pm`.
   - `hydra sessions list --from ${ISSUE_ID_2}` shows ONLY the original in-conversation session — no new headless `swe` session is dispatched in `pending`. The atomic `--clear-assignee` ensures the assignee transitions through `null` before the dispatcher's readiness check runs; any subsequent auto-assign by `SpawnSessionsAutomation` reattaches the assignment agent (`pm`), not `swe`, so no `swe` session is dispatched.
5. Optional re-spawn check: re-transition `${ISSUE_ID_2}` from `pending` back to `pair-development`. `pair-development`'s `on_enter.assign_to: agents/swe` re-sets the assignee; combined with `interactive: true` on the destination status, `AgentQueue` mints a fresh conversation `${CONV_ID_3}` (distinct id from `${CONV_ID_2}`) and the "Open Conversation" affordance re-appears in the header pointing at it. This validates [[d-ulhrefm]] §5 "Lifecycle" — re-entering an interactive status spawns a fresh conversation rather than reusing the closed one.
6. Cleanup: transition `${ISSUE_ID_2}` to `pending-release` (terminal) to leave the fixture in a clean state. `${CONV_ID_3}` should also close via `close_conversations_on_interactive_exit` since `pending-release` is non-interactive.

## Expected Results

**Bundle A — Interactive spawn and greet-user:**
- The `engineering-v2` project editor and every status-list / status-dropdown surface render `pair-development` with the "interactive" annotation chip.
- Transitioning `${ISSUE_ID}` from `inbox` to `pair-development` mints exactly one `Conversation` with `spawned_from == ${ISSUE_ID}`, status `Active`, and creator `agents/swe`. The headless-session branch is **not** taken — no headless `swe` session is spawned for the `pair-development` entry.
- The dashboard issue header renders the "Open Conversation" affordance pointing at the new conversation; the Related tab lists it in a Conversations subsection; the conversation page header shows an "Originated from [[${ISSUE_ID}]]" link.
- The spawned session carries `mode.type == "interactive"`, `mode.greet_user == true`, and `env_vars.HYDRA_ISSUE_ID == ${ISSUE_ID}`. The chat thread shows an assistant message before any user turn — the agent picked up the prompt and started work autonomously.

**Bundle B — Patch lifecycle through reviewer chain:**
- The in-conversation `swe` agent produces at least one patch on `dourolabs/hydra-test-fixture` and transitions `${ISSUE_ID}` from `pair-development` to `in-review`.
- The `pair-development` → `in-review` transition fires `apply_status_on_enter` (reassigns to `reviewer`, attaches `/forms/review.yaml`) **and** `close_conversations_on_interactive_exit` (closes `${CONV_ID}`). The closure is reflected on the issue header (affordance gone) and in the Related-tab Conversations subsection (row now shows status `Closed`).
- `reviewer` approves via the same form used by the headless reviewer chain; `${ISSUE_ID}` transitions to `pending-release`. No child `review-request` issue is created at any point — interactive mode preserves the same-issue review hand-off.
- A single issue's session list mixes one interactive `swe` session (`mode.type == "interactive"`, `mode.greet_user == true`, attached to `${CONV_ID}`) with one headless `reviewer` session (`mode.type == "headless"`) — the uniformity guarantee from [[d-ulhrefm]] §1 holds (patch and review semantics are mode-agnostic).

**Bundle C — Close-on-exit via status flip-out:**
- Transitioning `${ISSUE_ID_2}` from `pair-development` to `pending` (the flip-out also clears the assignee, so no follow-on dispatch confounds) closes `${CONV_ID_2}` via `close_conversations_on_interactive_exit` and removes the "Open Conversation" affordance from the issue header.
- Re-entering `pair-development` mints a **fresh** conversation `${CONV_ID_3}` (distinct id from `${CONV_ID_2}`); the closed conversation is not reused.
- Final flip to `pending-release` closes `${CONV_ID_3}` cleanly. The audit trail on the Related tab still shows all spawned conversations (`${CONV_ID_2}` and `${CONV_ID_3}`) with status `Closed`.

## Failure Modes

Report these explicitly so a failing run produces an actionable finding rather than a bare "timeout":

- **`pair-development` missing from the project editor or status dropdowns** → fixture seeding regression in `tests/e2e/run.sh` (the `engineering-v2` project body or the prompt push). Capture the project editor screenshot and the runner log.
- **Status dropdown shows `pair-development` without the "interactive" annotation chip** → PR 5 frontend regression ([[i-lfwhatzq]]). Capture the dropdown screenshot.
- **No conversation appears within 15s of the `inbox` → `pair-development` transition** → `AgentQueue::build_task` failed to dispatch through the interactive branch (PR 4 [[i-qmeoonyq]] regression) or `SpawnConversationSessionsAutomation` did not fire. Capture the server log at `/tmp/hydra-e2e/server.log` and the output of `hydra conversations list --output-format jsonl --status active`.
- **A headless `swe` session also spawned for the `pair-development` entry** → the two-branch dispatcher is not gating the headless path. PR 4 regression. Capture `hydra sessions list --from ${ISSUE_ID} --output-format jsonl`.
- **`Conversation.spawned_from` is null on the spawned conversation** → PR 2 [[i-aerntbio]] regression (the column migration or `create_conversation` parameter growth dropped the field).
- **`mode.greet_user` is `false` on the spawned session** → PR 3 [[i-bhjilhhs]] regression (the three-line `SpawnConversationSessionsAutomation` change). Confirm via `hydra --output-format jsonl sessions get ${SESSION_ID} | jq '.mode.greet_user'` (`sessions list` returns `SessionSummary`, which doesn't carry `mode` — use `get` for the full record).
- **No autonomous assistant message before the first user turn** → either `greet_user` is wrong (see above) or the chat-spawn `FirstMessage` did not emit. Capture the chat thread's first 60s of events.
- **`HYDRA_ISSUE_ID` env var missing on the in-conversation session** → PR 3 regression (the env-var derivation was meant to be duplicated, not refactored — see the round-2 reviewer note on [[i-xipntwnu]]).
- **Conversation remains `Active` after `pair-development` → `in-review`** → `close_conversations_on_interactive_exit` is not firing. PR 4 regression. Capture the event-bus trace.
- **Conversation remains `Active` after `pair-development` → `pending`** (same automation, different trigger path) → same regression as above, but specifically the manual flip-out direction. Distinguish from the agent-forward direction in the failure note.
- **Headless `swe` session spawns in `pending` despite `--clear-assignee`** → indicates either the `hydra issues update` atomic update is racing the dispatcher (unlikely but possible), or `pair-development.on_enter` is being re-applied on the re-entry side. Capture `hydra sessions list --from ${ISSUE_ID_2} --output-format jsonl` and the dispatcher tick log around the flip-out timestamp.
- **Re-entering `pair-development` reuses the closed conversation instead of spawning a fresh one** → [[d-ulhrefm]] §5 "Lifecycle" regression. Capture both conversation ids and the spawn-side log line.
- **`reviewer` chain produces a child `review-request` issue** → uniformity guarantee broken; the per-project same-issue review hand-off prompt slice did not apply inside the conversation session. Capture the child-issue list and the reviewer session's resolved `system_prompt`.
