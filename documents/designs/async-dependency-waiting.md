# Design: Async External Dependency Waiting in Playbooks

## Problem Statement

Metis playbooks automate multi-step deployment and operational processes. Many steps require waiting for an external dependency to complete before proceeding — for example:
- Waiting for a GitHub Actions workflow to build and push a Docker image
- Waiting for a Kubernetes deployment to become healthy
- Waiting for a PR to be reviewed/approved
- Waiting for a DNS record to propagate
- Waiting for a manual human approval

The question: what is the best mechanism for expressing and executing these "wait for external event" steps within metis's issue-driven agent orchestration model?

## Current System Constraints

Key facts from the metis codebase (as of 2026-02-11):

| Property | Value | Source |
|----------|-------|--------|
| Kubernetes job timeout | **None** (no `activeDeadlineSeconds`) | `job_engine/kubernetes_job_engine.rs` |
| Result submission timeout | 60 seconds after K8s job completes | `app_state.rs:1388` |
| Agent max retries per issue | 3 (default, configurable per-issue) | `config/mod.rs:498` |
| Claude Code `--max-turns` | **Not set** (unlimited) | `worker_commands.rs:183-193` |
| Agent spawn criteria | Issue must be Ready (no open blockers, no open children if InProgress) | `app_state.rs:issue_ready` |
| CLI poll-and-wait support | `metis jobs create --wait` polls every 2s indefinitely | `jobs/create.rs` |
| Issue dependency types | `BlockedOn` and `ChildOf` | `domain/issues.rs` |

## Options

### Option A: Agent Polling Loop (The "Sane Default")

**How it works**: Create a child issue (e.g., "Wait for Docker image build"). An agent spawns, runs a CLI polling loop (e.g., `gh run watch`, `kubectl rollout status`, or a custom `while` loop with `sleep`), and closes the issue when the condition is met. The parent playbook issue, which uses `ChildOf` dependencies, becomes Ready again when this child closes, and a follow-up agent picks up the remaining steps.

**Example playbook step**:
```
Create a child issue: "Wait for GitHub Actions workflow to complete on repo X.
  Use `gh run list --workflow=build.yml --branch=main --limit=1 --json status`
  to poll every 30 seconds. Close this issue when the status is 'completed'."
```

**Pros**:
- Works today with no code changes
- Simple to implement and reason about
- Agents already have secrets access (GitHub tokens etc.) via `JobSettings.secrets`
- Leverages existing issue dependency graph for sequencing
- Agent can do smart error handling (detect failure, report in progress notes)

**Cons**:
- **Resource waste**: A Kubernetes pod sits idle in a sleep loop consuming CPU/memory for potentially hours
- **Agent session limits**: Claude Code has API rate limits and usage quotas. A long polling loop may exhaust the session's context window or hit rate limits. Claude Code's `--print` mode (used in metis) runs as a single conversation turn, so the entire poll loop runs in one session
- **Cost**: Each poll iteration costs API tokens (if the agent reasons about each poll result); even at minimal cost per iteration, an hour of polling adds up
- **Fragility**: If the agent session crashes mid-poll (OOM, network error, API timeout), the retry count increments and the agent restarts from scratch — it won't resume the poll
- **No Kubernetes job deadline**: Since there's no `activeDeadlineSeconds`, a stuck poll loop would run forever. This is a risk even for non-polling agents, but poll loops make it more likely

**Mitigations**:
- Set `--max-turns` on Claude Code to prevent runaway sessions (requires a code change in `worker_commands.rs`)
- Use lightweight shell-only polling (avoid the agent reasoning each iteration): have the agent write a shell script and then `bash` it
- Add `activeDeadlineSeconds` to K8s job specs as a safety net (e.g., 2 hours)

**Resource estimate**: ~500m CPU, ~1Gi memory per polling pod. For a 30-minute wait with 30s poll interval, that's 60 poll iterations. If the agent just runs a bash `while` loop, token cost is near-zero after initial setup. If the agent reasons each iteration, ~60 API calls.

---

### Option B: Issue-Level Webhook / External Callback

**How it works**: Add a new API endpoint to metis-server that allows external systems to close (or update) an issue via HTTP callback. A playbook step creates a "wait" issue and registers a webhook URL. The external system (e.g., GitHub Actions via `repository_dispatch` or a custom webhook step) calls the metis API when done. The issue closes, unblocking the parent.

**Example flow**:
1. Playbook agent creates child issue "Wait for Docker image build" 
2. Agent writes the callback URL (e.g., `https://metis.example.com/v1/issues/{id}/close`) into the GitHub Actions workflow (or triggers a workflow that already knows the callback pattern)
3. Agent exits (issue stays open, parent blocked)
4. GitHub Actions workflow completes, calls the metis callback URL
5. Metis server closes the issue
6. Parent issue becomes Ready, next agent spawns

**Pros**:
- **Zero resource waste**: No agent or pod runs during the wait period
- **Instant notification**: No polling delay; the event triggers immediately
- **Scalable**: Can have many concurrent waits without consuming cluster resources
- **Clean separation**: The "wait" is handled by the event source, not by metis

**Cons**:
- **Requires code changes**: Need a new API endpoint (or extend existing issue update endpoint with a simpler auth model for webhooks)
- **External system coupling**: Requires the external system to know about metis and be configured to call back. Not all systems support webhooks (e.g., waiting for DNS propagation)
- **Authentication complexity**: Webhook endpoints need auth tokens; managing per-callback tokens adds complexity. Currently metis uses user-level OAuth tokens
- **Not universal**: Only works for systems that can make HTTP calls. Doesn't cover "wait for manual approval" or "wait for arbitrary CLI-observable condition"

**Implementation cost**: Medium. The metis API already has issue CRUD endpoints. Adding a `/v1/issues/{id}/callback` endpoint with a one-time token is straightforward. The bigger effort is integrating with each external system.

---

### Option C: Server-Side Polling Worker

**How it works**: Add a new background worker to metis-server (alongside `process_pending_jobs`, `monitor_running_jobs`, etc.) that polls external conditions on behalf of issues. Issues would gain a new field (e.g., `wait_condition`) specifying what to poll and how. The server worker polls periodically and closes the issue when the condition is met.

**Example**:
```json
{
  "wait_condition": {
    "type": "github_action",
    "repo": "dourolabs/metis",
    "workflow": "build.yml",
    "branch": "main",
    "poll_interval_seconds": 60,
    "timeout_seconds": 3600
  }
}
```

**Pros**:
- **Centralized and efficient**: One server-side process polls for all waiting conditions, no K8s pods needed
- **Built-in timeout**: Server can enforce timeouts and fail issues that wait too long
- **Reliable**: Server-side process is long-lived and doesn't have agent session limits
- **Observable**: Polling state is visible in the issue itself

**Cons**:
- **Significant code changes**: New domain concept, new background worker, new API fields
- **Limited flexibility**: Each new "condition type" requires server-side code. Can't easily wait for arbitrary CLI-observable conditions without building a plugin system
- **Server complexity**: The metis server becomes responsible for interacting with external systems (GitHub API, K8s API, etc.), requiring those credentials and dependencies
- **Tight coupling**: Server must understand the semantics of each external system

**Implementation cost**: High. This is essentially building a workflow engine feature into the server.

---

### Option D: Lightweight "Sentinel" Agent (Minimal Resource Variant of Option A)

**How it works**: Similar to Option A, but instead of using a full Claude Code agent session, create a specialized "sentinel" agent type that runs a simple shell script (no LLM involved) in a minimal container. The sentinel script polls the condition and calls `metis issues update` when done.

**Example**:
```bash
#!/bin/bash
# sentinel script: wait for GitHub Actions workflow
while true; do
  STATUS=$(gh run list --workflow=build.yml --branch=main --limit=1 --json status -q '.[0].status')
  if [ "$STATUS" = "completed" ]; then
    metis issues update $METIS_ISSUE_ID --status closed
    exit 0
  fi
  sleep 30
done
```

The issue would specify a `sentinel_script` field (or use a different issue type / agent type) that bypasses the LLM entirely and just runs the script.

**Pros**:
- **Minimal resource usage**: Tiny container (alpine + curl/gh), ~10m CPU, ~32Mi memory
- **No LLM cost**: Zero API token usage
- **Flexible**: Any CLI-observable condition can be checked
- **Simple**: Shell scripts are easy to write and debug
- **Reliable**: No agent session limits, no context window concerns

**Cons**:
- **Requires code changes**: Need a new "sentinel" task type or agent type that skips LLM invocation
- **Less intelligent error handling**: A shell script can't reason about unexpected failures the way an LLM agent can
- **Script management**: Where do sentinel scripts live? How are they versioned? Need a convention
- **Still consumes a pod**: Even a minimal pod is a K8s resource. At scale (many concurrent waits), this could be a concern

**Implementation cost**: Medium. Requires adding a sentinel execution mode to the job engine (bypass LLM, just run a script) and a way to specify the script in issue/job settings.

---

### Option E: Hybrid — Agent Sets Up Webhook, Falls Back to Sentinel

**How it works**: Combine Options B and D. The playbook agent first attempts to set up a webhook callback (if the external system supports it). If that's not possible, it falls back to creating a sentinel polling issue. This gives the best of both worlds: zero-cost waiting when webhooks are available, minimal-cost polling when they're not.

**Pros**:
- **Optimal resource usage**: Zero cost when webhooks work, minimal cost when polling
- **Universal**: Handles any external dependency type
- **Graceful degradation**: If webhook setup fails, sentinel still works

**Cons**:
- **Most complex to implement**: Requires both webhook infrastructure and sentinel infrastructure
- **Two code paths to maintain**: More surface area for bugs
- **Overkill for current scale**: If metis only has a handful of concurrent waits, simpler options suffice

---

## Comparison Matrix

| Criterion | A: Agent Poll | B: Webhook | C: Server Poll | D: Sentinel | E: Hybrid |
|-----------|:---:|:---:|:---:|:---:|:---:|
| Works today (no code changes) | **Yes** | No | No | No | No |
| Resource efficiency | Low | **High** | **High** | Medium | **High** |
| LLM token cost | Medium-High | **Zero** | **Zero** | **Zero** | **Zero** |
| Flexibility (arbitrary conditions) | **High** | Low | Low | **High** | **High** |
| Implementation complexity | **Low** | Medium | High | Medium | High |
| Reliability (long waits) | Low | **High** | **High** | **High** | **High** |
| Universality (any external system) | **High** | Low | Low | **High** | **High** |
| Observability | Medium | Medium | **High** | Medium | Medium |

## Recommendation

**Short-term (now)**: Use **Option A (Agent Polling)** with mitigations:
1. Have the agent write and execute a pure-bash polling script (no LLM reasoning per iteration)
2. Set a reasonable `--max-turns` on Claude Code to prevent runaway sessions
3. Document this pattern as a playbook convention

**Medium-term (next sprint)**: Implement **Option D (Sentinel Agent)** as the primary mechanism:
1. Add a "sentinel" execution mode to the job engine that runs a shell command without LLM
2. Use a minimal container image (alpine + metis CLI + common tools like `gh`, `kubectl`, `curl`)
3. Add `activeDeadlineSeconds` to K8s job specs for safety (configurable per-issue, default 2h)
4. Sentinel issues are created by playbook agents and specify the poll script in their description or a new field

**Long-term (future)**: Add **Option B (Webhooks)** for high-frequency integrations (GitHub Actions → metis) where polling is wasteful.

This progression lets the team start immediately with no code changes, then reduce cost/risk with a targeted code change, and add webhook support as needed for specific integrations.

## Open Questions

1. **Claude Code session limits**: What is the practical maximum runtime for a Claude Code `--print` session before it hits rate limits or context exhaustion? This determines how long Option A can reliably poll. (Initial research suggests a 5-hour rolling session window, but this may vary by plan tier.)
2. **Sentinel container image**: Should sentinels reuse the existing worker image (large, has all dependencies) or use a minimal image? Minimal is better for resource usage but requires building/maintaining another image.
3. **Script specification**: For Option D, should the polling script be embedded in the issue description, stored as a document, or referenced from the playbook? Embedding is simplest but less reusable.
4. **Timeout behavior**: When a wait times out, should the issue be marked as `Failed` or left open for human intervention? Failing is cleaner for automation; leaving open is safer for deployments.
5. **Kubernetes resource quotas**: Does the cluster have resource quotas that would limit the number of concurrent polling pods? (Current cluster is single-node with limited resources.)

## References

- [MCP SEP-1686: Tasks proposal](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1686) — emerging standard for async task primitives in agent systems
- [GitHub Actions `repository_dispatch` event](https://docs.github.com/actions/learn-github-actions/events-that-trigger-workflows) — mechanism for external event triggers
- [Agentfield async execution patterns](https://agentfield.ai/docs/core-concepts/async-execution) — reference implementation of no-timeout async workflows
- Metis codebase: `metis-server/src/background/spawner.rs`, `metis-server/src/app/app_state.rs`, `metis/src/worker_commands.rs`