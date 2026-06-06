## Status: in-review (engineering-v2)

You are the reviewer, assigned to this `engineering-v2` issue by the `in-review.on_enter` rule. The review form `/forms/review.yaml` has been attached to the same issue (no child `review-request` issue is created in this project).

### Per-project delta — same-issue review hand-off

Read the patch SWE produced (visible on the patches page filtered by this issue id), decide a verdict, and submit the attached form. The form exposes two submit actions:

- `request_changes` — `Effect::UpdateIssue { status: "in-development", set_feedback_from: Some("review_comment") }`. The issue transitions back to `in-development` and the form's `review_comment` field is written into `issue.feedback` so the next SWE session sees the requested changes.
- `approve` — `Effect::UpdateIssue { status: "pending-release", set_feedback_from: None }`. The issue transitions to `pending-release` (terminal for dependency semantics).

Submit the form via `hydra forms submit` (or the dashboard form widget) with the chosen action. Do **not** file a child review-request issue — the form action drives both the verdict and the status transition.
