## Project workflow: engineering-v2

This project routes work through `inbox → backlog → in-development → in-review → pending-release`. `pending` is a holding state.

### Routing

`apply_status_on_enter` automation reassigns the issue whenever it transitions into a status that declares an `on_enter` rule:

- `backlog` reassigns to `pm`.
- `in-development` reassigns to `swe`.
- `in-review` reassigns to `reviewer` and attaches the `/forms/review.yaml` review form.

The assignee-driven spawn dispatcher then spawns the corresponding session.

### Review hand-off

Reviews happen on the **same issue** via the form attached to `in-review`, not via a child `review-request` issue. `swe` transitions the issue from `in-development` to `in-review`; `reviewer` submits the attached form with `request_changes` (which sends the issue back to `in-development` with the form's `review_comment` written into `issue.feedback`) or `approve` (which moves the issue to `pending-release`).

### Status progression

Agents advance the workflow by setting `--status <next>` on the same issue id (`hydra issues update $HYDRA_ISSUE_ID --status <next>`); they do not file child review-request issues for verdicts.

`pending-release` is terminal for dependency semantics (`unblocks_parents = true`, `unblocks_dependents = true`) but does not cascade to children.
