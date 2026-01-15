# Operations

## CI failure auto-review flow
- The GitHub poller inspects open patches with PR metadata and records the latest CI status on every cycle.
- When CI reports a failure and `enable_ci_failure_autoreview` is enabled, the poller posts a review with the failing check, summary, and logs URL, then closes the PR to keep the queue clean.
- Reviews are idempotent: if the expected CI failure review already exists, no additional action is taken.

## Configuration
- Location: `metis-server/config.toml` under `[background.github_poller]`.
- Flag (defaults to `false`): `enable_ci_failure_autoreview = true` to turn on CI failure reviews/closures.
- The poller still records CI status metadata even when disabled. Ensure the target service repository is configured with a GitHub token so the poller can read PR status and write reviews when enabled.

## Observability & alerts
- Metrics endpoint: `GET /metrics` (Prometheus format).
  - `github_ci_poll_results_total{state}` counts CI poll results by state (`pending|success|failed`).
  - `github_ci_autoreview_actions_total{action,result}` counts review/close/alert attempts and outcomes (`success|error|disabled|duplicate`).
- Logs: CI poll results and auto-review actions are logged with patch/PR identifiers; failures to post reviews or close PRs emit `error` logs with `alert=true`.
- Alerting: hook alerts to the `github_ci_autoreview_actions_total{action=\"alert\",result=\"error\"}` counter or to `alert=true` error logs to surface review/closure failures.
