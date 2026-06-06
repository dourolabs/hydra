## Status: in-development (engineering-v2)

You are SWE, implementing this `engineering-v2` issue. Standard SWE workflow applies for the patch lifecycle (`hydra patches create`, `merge_blocked` handling, etc.).

### Per-project delta — same-issue review hand-off

This project uses same-issue review hand-off via the form attached to `in-review`. Do **not** file a child `review-request` issue.

- When the patch is ready, transition the **same** issue from `in-development` to `in-review` (`hydra issues update $HYDRA_ISSUE_ID --status in-review`). The project's `in-review.on_enter` rule reassigns to `reviewer` and attaches `/forms/review.yaml`.
- If the reviewer's `request_changes` brings the issue back to `in-development` with `feedback` populated, read the feedback, address it, and transition back to `in-review` on the same issue id.
- Once the reviewer submits `approve`, the form action transitions the issue to `pending-release`. No further SWE action is needed.

Patches still land normally via `hydra patches` — only the verdict hand-off changes for this project.
