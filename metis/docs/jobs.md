# `metis jobs`

`metis jobs` manages long-running orchestration workers. Use it to launch Codex/Claude-backed agents, inspect their output, and mirror their execution locally.

## Authentication & global flags

All `metis jobs` subcommands honor global CLI flags (e.g. `--server-url`, `--token`, `--output-format jsonl`). Pretty output shows a compact table and truncates notes; use JSONL when feeding job results back into scripts.

### Environment variables

- `METIS_ISSUE_ID`: default issue to associate with new jobs and `worker-run` tracking branches.
- `METIS_ID`: populated inside agent pods so downstream tools can tag artifacts. When you call `metis jobs logs <ISSUE_ID>`, the CLI finds the latest job spawned from that issue.
- `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`: picked up by `jobs worker-run` unless you override with explicit flags.

## Typical workflow

1. Launch a worker tied to a repo prompt.
   ```bash
   metis jobs create --repo dourolabs/metis --rev main \
     --var MODEL=gpt-4.1 --var MODE=analyze --wait "triage flaky tests"
   ```
   `--wait` streams the server-side logs immediately; omit it to fire-and-forget.
2. Tail ongoing output later.
   ```bash
   metis jobs logs t-job123 --watch
   ```
   The `--watch` flag keeps streaming until the job finishes, while allowing you to pass either a job id (`t-…`) or an issue id (`i-…`).
3. List active or recent work.
   ```bash
   metis jobs list --limit 20 --from i-fdmrzs
   ```
   `--from` restricts results to jobs launched from a specific issue id.
4. Reproduce the context locally.
   ```bash
   metis jobs worker-run t-job123 ./replay \
     --openai-api-key $OPENAI_API_KEY --anthropic-api-key $ANTHROPIC_API_KEY
   ```
   `worker-run` fetches the bundle, applies the nearest build cache (same behavior as running `metis caches apply --nearest`), injects your API keys, and pushes updates back to the job.
5. Clean up runaway jobs.
   ```bash
   metis jobs kill t-job123
   ```

## Subcommands

### Create

```bash
metis jobs create [--wait] [--repo <NAME>|<GIT_URL>] [--rev <REV>] \
  [--image <IMAGE>] [--var KEY=VALUE ...] [--issue-id <ISSUE_ID>] "prompt"
```

- `--repo` accepts an internal service repo (`dourolabs/service`) or a full git URL. Combine with `--rev` (defaults to `main`) when pinning a commit or branch.
- `--var` sets job environment variables (e.g. `MODEL`, `GITHUB_TOKEN`). The CLI automatically injects `PROMPT` based on the trailing quoted text.
- `--image` overrides the worker Docker image for targeted debugging.
- `--wait` prints the new job row, streams logs, and blocks until completion.
- `--issue-id` defaults to `METIS_ISSUE_ID`; associating jobs keeps `jobs logs <ISSUE>` working and surfaces activity on the dashboard.

### List

```bash
metis jobs list [--limit <COUNT>] [--from <ISSUE_ID>]
```

Shows the newest jobs in the configured namespace. Increase `--limit` (default 10) to page through history. `--from` filters to the ID that originally spawned the work, helping you separate personal jobs from team traffic.

### Logs

```bash
metis jobs logs <JOB_ID|ISSUE_ID> [--watch]
```

Fetches log output for an individual job (`t-job…`) or resolves the newest job tied to an issue (`i-…`). `--watch` keeps the stream open until the job finishes; without it, the CLI prints the latest buffered log chunk and exits.

### Kill

```bash
metis jobs kill <JOB_ID>
```

Immediately sends a termination signal to the worker pod, then re-fetches the job record so you can see the final status and notes. Use this when a run is clearly stuck or misconfigured.

### Worker-run

```bash
metis jobs worker-run <JOB_ID> <PATH> \
  [--issue-id <ISSUE_ID>] [--openai-api-key <KEY>] \
  [--anthropic-api-key <KEY>] [--claude-code-oauth-token <TOKEN>]
```

Downloads the job context into `<PATH>` (which must be empty), restores cached build artifacts, and then executes the original prompt via Codex/Claude using the provided credentials. After the local run, the CLI pushes branches and status updates back to the Metis server so reviewers can inspect artifacts. Override the API key flags to test different providers while keeping your defaults untouched. Provide `--issue-id` (or rely on `METIS_ISSUE_ID`) to keep branch naming consistent across retries.
