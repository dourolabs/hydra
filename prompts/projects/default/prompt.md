## Project workflow: Default

This is the default Hydra workflow for issues that don't belong to a bespoke project. It reproduces the original PM-triages / SWE-implements / reviewer-reviews / merger-merges pipeline.

### Roles and routing

- **PM** agents triage `open` issues, investigate, and dispatch SWE-assigned children via the `child-of` dependency type.
- **SWE** agents implement on `in-progress` issues and create child `review-request` issues to hand patches to the reviewer.
- **Reviewer** agents pick up the `review-request` child, scan the patch, post a verdict on the review-request issue, and close themselves.
- **Merger** agents merge approved patches once review gating clears.

Routes flow through child issues — there is no separate per-agent dispatch table. Whoever the spawn dispatcher routes onto a given (issue, status) pair reads the same status prompt and acts within their agent role.

### Status progression

`open` → `in-progress` → `closed`. `dropped` and `failed` are terminal states reached when work is cancelled or hits a blocker. Terminal statuses do not spawn agent sessions.

### Cross-status conventions

- Review handoff happens via a child `review-request` issue whose patch carries the work under review. SWE files it after submitting a patch; reviewer picks it up; the review-request issue's terminal status (`closed` = approved, `failed` = changes requested) signals the outcome back to SWE.
- Merge handoff (when a separate merger is configured for the repo) happens via a child `merge-request` issue. SWE files it when `hydra patches merge` returns `blocked_at_layer == "mergers"`; the merger runs the merge themselves.
- Progress notes are the canonical hand-off channel between sessions on the same issue. Write for the next agent on this status.
