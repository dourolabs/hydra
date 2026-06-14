## Status: in-progress

The issue is being actively worked. SWE is the usual occupant; the action depends on whether the issue already has patches.

### Debug-only / diagnosis-only issues

Some issues are scoped to **diagnosis only** — they ask for a root-cause analysis without an implementation. The issue body usually says so explicitly ("Scope: diagnosis only — do not fix yet", "RCA only", "produce a diagnosis", etc.).

For these:

1. Investigate as instructed. Do NOT write or submit a patch.
2. Write the diagnosis directly into a comment via `hydra issues update $HYDRA_ISSUE_ID --comment "..."` (or `hydra issues comment $HYDRA_ISSUE_ID --body "..."`). Include the root cause, file / function pointers, and what change would fix it.
3. When the diagnosis is complete, set status to `closed`. The "do not close until a patch is submitted and merged" rule does NOT apply here — the diagnosis IS the deliverable.

If you're unsure whether an issue is debug-only, default to treating it as implementation work unless the body clearly says otherwise.

### Implementation work — no patches yet

1. Implement the change. You are already on a branch for this issue — commit your changes there.
2. Submit via `hydra patches create`, assigning to the issue's `creator` (from `hydra issues get`).

### Implementation work — patches exist

- **Merged**: task may be done. Review feedback for follow-ups. Create follow-ups as independent items with `hydra issues create` (short, informative titles ≤ ~70 chars). Do NOT use `--deps child-of:$HYDRA_ISSUE_ID` for follow-ups — reserve `child-of` for sub-tasks of the current issue.
- **ChangesRequested** (review left without closing the PR): address all comments, then reopen with `hydra patches update <PATCH_ID> --status Open` (you MUST pass `--status Open` to trigger another review — same patch id).
- **Open with approved review**: merge with `hydra patches merge <patch-id>`.
- **Closed**: significant feedback — rework the patch and resubmit a new one.

### Handling `merge_blocked` errors

When `hydra patches merge` fails with `code: merge_blocked`, parse the structured JSON response and dispatch on `blocked_at_layer`. Each merge attempt reveals exactly one layer; drive purely off the next response, never speculatively ahead.

- **`blocked_at_layer == "reviews"`** (one or more `missing_approvals` reasons): for each reason in `reasons[]`, create ONE review-request issue with `hydra issues create "<description>" --type review-request --title "<title_hint from the reason>" --assignee <pick one from suggested_action.assign_to_one_of> --deps child-of:$HYDRA_ISSUE_ID`. End the session. The next SWE reinvocation (after the RR closes) retries `hydra patches merge`.
- **`blocked_at_layer == "mergers"`** (the single `not_in_mergers` reason): create ONE merge-request issue with `hydra issues create "<description>" --type merge-request --title "<title_hint from the reason>" --assignee <pick one from suggested_action.assign_to_one_of> --deps child-of:$HYDRA_ISSUE_ID`. End the session. The merger runs `hydra patches merge` themselves — THIS SWE does NOT retry; the work is handed off.
- NEVER file MR issues while `blocked_at_layer == "reviews"` — reviews are gated ahead of mergers, and a `not_in_mergers` reason will not appear until every reviewer group is satisfied. NEVER speculatively file ahead of the next `merge_blocked` response; it is the only source of truth for what to do next.

Once all needed changes are merged and follow-ups are complete, set status to `closed`.

### Use `failed` sparingly

Use `failed` only for fundamental blockers (infeasible approach, contradictory requirements, blocking technical limit). Do not use it for transient/retryable errors. If the task changes the codebase, do NOT close until a patch is submitted and merged.
