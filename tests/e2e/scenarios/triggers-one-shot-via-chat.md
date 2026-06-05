# Scenario: Triggers One-Shot Via Chat

**ID:** triggers-one-shot-via-chat
**Category:** agent-coordination
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed); `hydra triggers` CLI present in the test instance (PR 6 / [[i-fhmpscam]] merged); chat agent's prompt allows `hydra triggers create` and at least `hydra triggers get` (parent follow-up); test fixture repo `dourolabs/hydra-test-fixture` registered (add-github-repo scenario passed)
**Estimated duration:** 3 minutes

## Description

Verify the full user → chat → one-shot trigger → issue → graph-edge path. The tester role-plays a user in the Chat UI, asks the chat agent to set up a one-shot trigger that fires roughly 45 seconds in the future and creates a specific issue, then polls until the issue lands and verifies (a) the issue fields, (b) the `ActorRef::Trigger { on_behalf_of: Some(User) }` actor on the create event, and (c) the `Trigger -created-> Issue` graph edge. This exercises the triggered-actions runtime (PR 5 [[p-fnpbknnu]] and PR 7 [[p-xvosxrnb]]) end-to-end through the chat surface that real users hit.

## Steps (via dashboard, Playwright MCP)

1. Generate a `<RUN_ID>` — an 8-char timestamp/random string the tester picks at the start of the run, so the target issue title is unique and pollable across re-runs (e.g. `20260604` plus a 2-char nonce, or a millisecond suffix).
2. Open `http://localhost:8080/chat`. Click the "New Chat" button.
3. Send a single user message of this exact form (verbatim — the recognizable title is what the verification polls for):

   > Please create a one-shot trigger that fires roughly 45 seconds from now and creates an issue titled `E2E one-shot trigger smoke <RUN_ID>` (description: "Verification target for the triggers-one-shot-via-chat e2e scenario"; assignee: `users/swe`; status: `open`; repository: `dourolabs/hydra-test-fixture`; issue type: `task`). Reply with just the new trigger id and the scheduled fire time when done.

4. Wait up to 60s for the assistant reply. Parse the reply for a `t-…` trigger id; record both the trigger id and the scheduled fire time (`scheduled_at`) returned by the chat agent. If no `t-…` id appears within 60s, treat this run as `skipped` (see Failure Modes below), not `failed`.
5. Verify via the dashboard that the trigger exists: navigate to `/triggers/<trigger-id>` and assert:
   - The schedule renders as a `Once { at }` value roughly 45s in the future (within a small slack window from `scheduled_at`).
   - The single action is `CreateIssue` and the rendered title contains `<RUN_ID>`.
   - The creator is shown as the chat user.
6. Begin polling for the created issue. The `ScheduledTriggerWorker` tick is 10s and the design's verification window allows one tick of drift (see [[d-jpjycat]] §4 / §4.6), so the issue should land within `scheduled_at + ~10s`; cap the poll at **90s wall-clock** from message-send. Poll both surfaces:
   - Dashboard: navigate to `/issues` and filter by title-contains `<RUN_ID>`.
   - API: `curl -s http://localhost:8080/v1/issues | jq '.issues[] | select(.title | contains("<RUN_ID>"))'`.
7. Once the issue exists, capture its `issue_id` and assert all of:
   - `title == "E2E one-shot trigger smoke <RUN_ID>"`.
   - `issue_type == "task"`.
   - `status == "open"`.
   - `assignee == users/swe`.
   - `session_settings.repo_name == "dourolabs/hydra-test-fixture"`.
   - The actor recorded on the create event is `ActorRef::Trigger { trigger_id: <trigger-id>, on_behalf_of: Some(ActorId::User(<chat user>)) }`. Verify via the issue-detail activity log in the dashboard if it renders the typed actor; otherwise fall back to `GET /v1/issues/<id>` and inspect the create-event `actor` field. If the dashboard renderer is incomplete for `ActorRef::Trigger`, screenshot what it renders and treat the JSON check as authoritative.
8. Assert the graph edge exists: `curl -s "http://localhost:8080/v1/relations?source_id=<trigger-id>&rel_type=created" | jq '.relations'` returns exactly one row with `target_id == <issue-id>` and a `created_at` timestamp inside the fire window (between `scheduled_at` and `scheduled_at + 15s`).
9. Cleanup: click "End Chat" in the chat header to end the conversation. The created trigger, issue, and edge stay — they are audit evidence for the run.

## Expected Results

- The chat agent acknowledged the request and replied within 60s with a `t-…` trigger id and a scheduled fire time.
- The trigger detail page renders the schedule as `Once { at }` ~45s in the future, the single `CreateIssue` action with a title containing `<RUN_ID>`, and the chat user as the creator.
- The target issue appears within the 90s polling window with `title == "E2E one-shot trigger smoke <RUN_ID>"`, `issue_type == "task"`, `status == "open"`, `assignee == users/swe`, and `session_settings.repo_name == "dourolabs/hydra-test-fixture"`.
- The actor on the create event is `ActorRef::Trigger { trigger_id: <trigger-id>, on_behalf_of: Some(ActorId::User(<chat user>)) }` (per dashboard activity log if it renders the typed actor, otherwise via `GET /v1/issues/<id>`).
- The relations endpoint returns exactly one `Trigger -created-> Issue` row with `target_id == <issue-id>` and `created_at` inside `[scheduled_at, scheduled_at + 15s]`.
- No JavaScript errors in the chat page or the trigger detail page.

## Failure Modes

Report these explicitly so a failing run produces an actionable finding rather than a bare "timeout":

- **No `t-…` id in chat reply within 60s** → the chat agent does not know about triggers yet (its prompt/allowlist gate is not met). Treat the run as `skipped`, not `failed`, and note which gate is missing.
- **Trigger detail page errors or shows the schedule incorrectly** → frontend regression on PR 7 [[p-xvosxrnb]]. Capture the page state.
- **Issue never appears within 90s** → the `ScheduledTriggerWorker` is not ticking or `Action::run` errored. Capture the server logs at `/tmp/hydra-e2e/server.log`.
- **Actor on the create event is `ActorRef::Authenticated { … }` instead of `ActorRef::Trigger { … }`** → the worker is attributing writes wrong; PR 5 [[p-fnpbknnu]] regression.
- **Relations query returns 0 rows** → the follow-up `Store::add_relationship(trigger_id, issue_id, RelationshipType::Created)` call is silently failing (PR 5 §4.2 invariant violated). The issue itself will still exist, so distinguish "issue missing" from "edge missing" in the failure note.
