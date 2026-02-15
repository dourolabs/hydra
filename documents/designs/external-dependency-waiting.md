# Design: External Dependency Waiting in Playbook Automation

## Problem Statement

Metis playbooks (markdown documents that automate deployments and other processes) often need to wait for external dependencies to complete before proceeding. For example, a deployment playbook may need to wait for a GitHub Action to build and push a Docker image before updating a Kubernetes deployment to use the new image.

The system currently lacks a standardized mechanism for expressing and handling these external waits. This document evaluates several options and recommends an approach.

## Current System Context

### Relevant Architecture

- **Issue dependency graph**: Issues support `blocked-on` and `child-of` relationships. An issue is "Ready" (and eligible for agent spawning) only when all its `blocked-on` dependencies are in a terminal state (Closed, Dropped, Rejected, Failed).
- **Agent spawning loop**: The `RunSpawnersWorker` checks every 3 seconds for ready issues and spawns agents for them. No explicit timeout exists on this loop.
- **Agent sessions**: Agents run as Kubernetes Jobs via Claude Code CLI with `--print --dangerously-skip-permissions`. There is no `activeDeadlineSeconds` or `--max-turns` limit set. Sessions run until Claude Code finishes naturally, the job is killed, or the pod is evicted.
- **Git state preservation**: Agent work is preserved across sessions via tracking branches (`metis/<issue-id>/head`), enabling multi-session workflows.
- **GitHub polling**: A `GithubPollerWorker` runs every 60 seconds and polls GitHub for PR/CI status updates on open patches.

### Key Constraints

1. **Claude Code session costs**: Each agent session consumes API tokens. Long idle waits are wasteful.
2. **Claude Code session limits**: Anthropic enforces a 5-hour rolling window and weekly usage caps. Long-running polling sessions consume these quotas.
3. **Kubernetes pod resources**: Each agent pod uses 8 CPU / 24Gi memory (current cluster config). Keeping pods alive for long waits wastes cluster resources.
4. **Agent reliability**: Claude Code sessions can end unexpectedly. A polling agent might exit before the wait completes.
5. **Max retries**: Issues have a `max_tries` limit (default 3). Each agent session counts as one try.

## Options

### Option A: Agent-Based Polling (Baseline)

**Description**: Create an issue assigned to an agent with instructions to use CLI tools (e.g., `gh`, `curl`) to poll an external resource until it completes, then mark the issue as closed.

**How it works**:
1. Playbook agent creates a child issue: "Wait for GitHub Action X to complete"
2. Agent spawns, runs a bash loop: `while ! gh run view $RUN_ID --json status | jq -e '.status == "completed"'; do sleep 30; done`
3. Agent marks issue as closed once the condition is met
4. Downstream issues (blocked-on this one) become ready

**Pros**:
- No system changes required; works today
- Fully flexible -- agent can check any external resource via CLI tools
- Secrets are available via Kubernetes secret mounts

**Cons**:
- **Resource waste**: An 8-CPU / 24Gi pod sits idle running `sleep 30` in a loop
- **Token consumption**: Claude Code consumes API tokens even when idle. A 1-hour wait could consume significant token budget
- **Session reliability**: Claude Code sessions may time out, get rate-limited, or be interrupted. If the session ends before the wait completes, a retry burns another `max_tries` attempt
- **Session limits**: Extended waiting eats into the 5-hour rolling window and weekly caps
- **Unpredictable behavior**: LLM agents may not reliably implement tight polling loops -- they might misinterpret when to stop or take unexpected actions during the wait

**Estimated cost for 1-hour wait**: ~$5-15 in API tokens + cluster resources

### Option B: Lightweight Polling Job (No LLM)

**Description**: Add a new job type to metis that runs a simple shell script (no LLM) to poll an external resource, then updates the issue status via the metis CLI.

**How it works**:
1. Playbook agent creates a "wait" issue with a special job type or a dedicated CLI command
2. Metis spawns a lightweight Kubernetes Job (minimal container, ~100m CPU / 128Mi memory) that runs a user-specified shell command in a polling loop
3. When the command exits 0, the job marks the issue as closed via `metis issues update`
4. Downstream issues unblock

**Implementation sketch**:
```
metis issues create "Wait for GH Action" \
  --type task \
  --assignee wait-agent \
  --wait-command "gh run view $RUN_ID --json status -q '.status'" \
  --wait-value "completed" \
  --wait-interval 30 \
  --wait-timeout 3600 \
  --secrets github-token
```

Or more generically, a new CLI command:
```
metis jobs wait \
  --issue-id i-xyz \
  --command "gh run view $RUN_ID --json status -q '.status'" \
  --expect "completed" \
  --interval 30s \
  --timeout 1h
```

**Pros**:
- No LLM token consumption during the wait
- Minimal resource usage (tiny pod)
- Deterministic behavior -- shell script, not LLM improvisation
- Can add timeout and retry logic with well-defined semantics
- Secrets available via the same Kubernetes secret mechanism

**Cons**:
- Requires server-side changes (new job type or new command handling)
- Still uses a running pod (though much smaller)
- User must express the wait condition as a shell command -- less flexible than natural language
- Need to handle edge cases: what if the command never succeeds? What if the pod is evicted?

**Estimated cost for 1-hour wait**: ~$0.01 in compute (no API tokens)

### Option C: Server-Side Webhook Receiver

**Description**: Add a webhook endpoint to the metis-server that external systems (e.g., GitHub Actions) can call to mark an issue as complete.

**How it works**:
1. Playbook agent creates a "wait" issue and registers a webhook condition
2. The agent configures the external system (e.g., GitHub Action) to POST to the metis webhook when done
3. Metis-server receives the webhook, validates it, and closes the issue
4. Downstream issues unblock

**Implementation sketch**:
```
POST /v1/webhooks/issue-complete
{
  "issue_id": "i-xyz",
  "secret": "abc123"
}
```

Or integrated with GitHub's `workflow_run` webhook event.

**Pros**:
- Zero resource consumption during the wait -- no running pods
- Near-instant response (no polling delay)
- Clean event-driven architecture

**Cons**:
- Requires the metis-server to be reachable from external systems (currently behind Tailscale VPN -- this is a hard blocker without network changes)
- Requires configuring webhooks on external systems, which adds complexity and fragility
- Each external system integration needs custom webhook handling
- Security considerations: authenticating incoming webhooks, preventing replay attacks
- More complex to implement and debug than polling

**Estimated cost for 1-hour wait**: ~$0 (no pods, no tokens)

### Option D: Server-Side Polling Worker

**Description**: Add a new background worker to metis-server that polls external resources on behalf of issues, similar to the existing `GithubPollerWorker`.

**How it works**:
1. Playbook agent creates a "wait" issue with a poll configuration stored in issue metadata
2. A new `ExternalDependencyPollerWorker` runs in the metis-server process, checking registered conditions periodically
3. When a condition is met, the worker closes the issue
4. Downstream issues unblock

**Implementation sketch**:
Issues would store poll conditions in a new field:
```json
{
  "wait_condition": {
    "type": "github_action_run",
    "repo": "dourolabs/metis",
    "run_id": 12345,
    "expected_conclusion": "success"
  }
}
```

The server polls these conditions using the GitHub API (or other configured backends).

**Pros**:
- Zero extra pods -- runs in the server process
- Leverages existing background worker infrastructure (scheduler, backoff)
- Can share rate-limited API clients (e.g., GitHub API)
- Centralized monitoring and logging

**Cons**:
- Requires server-side changes for each new external system type
- Tightly couples the server to specific external systems (GitHub, Docker Hub, etc.)
- Less flexible -- new wait types require code changes
- Server resource consumption grows with number of active waits
- Requires secrets management in the server process

**Estimated cost for 1-hour wait**: ~$0 (shared server resources)

### Option E: Issue Dependency + Existing GitHub Poller (Minimal Change)

**Description**: Leverage the existing `GithubPollerWorker` and issue dependency system without any new code. The agent sets up issue dependencies such that downstream work is naturally gated.

**How it works**:
1. Playbook agent creates a patch (PR) that triggers the GitHub Action
2. The existing GitHub poller monitors the PR's CI status
3. Agent creates downstream issues that are `blocked-on` the current issue
4. The current issue stays InProgress until the CI passes and the patch is merged
5. Agent session ends, and the next spawn picks up when CI results are available

**Pros**:
- No system changes required
- Leverages existing infrastructure
- Natural integration with the PR/CI workflow

**Cons**:
- Only works for GitHub CI/PR-based dependencies, not arbitrary external events
- Requires the external dependency to be modeled as a PR check -- not always possible
- Agent re-spawn still costs LLM tokens to "check in" on progress
- Doesn't generalize well to non-GitHub dependencies (Docker Hub, external APIs, etc.)

**Estimated cost for 1-hour wait**: ~$1-3 per check-in spawn

## Comparison Matrix

| Criterion | A: Agent Polling | B: Lightweight Job | C: Webhook | D: Server Poller | E: Existing Infra |
|-----------|:---:|:---:|:---:|:---:|:---:|
| No code changes needed | Yes | No | No | No | Yes |
| Works for arbitrary external deps | Yes | Yes | Partial | Partial | No |
| Resource efficient | No | Mostly | Yes | Yes | Mostly |
| Token efficient | No | Yes | Yes | Yes | Partial |
| Deterministic/reliable | No | Yes | Yes | Yes | Partial |
| Complexity to implement | None | Medium | High | High | None |
| Complexity to use (playbook author) | Low | Low | High | Medium | Low |
| Handles long waits (1hr+) | Risky | Yes | Yes | Yes | Yes |
| Generalizes to future needs | Partial | Yes | Partial | Partial | No |

## Recommendation

**Primary recommendation: Option B (Lightweight Polling Job)**, with Option E as a complement for GitHub-specific workflows.

### Rationale

1. **Option A is too expensive and unreliable** for waits longer than a few minutes. An LLM agent is the wrong tool for running `sleep 30` in a loop.

2. **Option B provides the best balance** of flexibility, cost, reliability, and implementation effort:
   - It works for any external dependency expressible as a shell command
   - It costs almost nothing to run (tiny pod, no LLM tokens)
   - It's deterministic and predictable
   - The implementation is moderate -- a new job type or CLI command, plus a small container image
   - It naturally integrates with the existing issue/dependency system

3. **Option C (webhooks) is architecturally cleaner** but impractical given the current VPN-only network topology. It would require exposing an endpoint publicly and handling authentication. This could be a future enhancement if metis-server gets a public API gateway.

4. **Option D (server-side poller) is viable** but creates tight coupling between the server and external systems. Each new wait type requires server code changes. Better to keep the server generic and push polling logic to job containers.

5. **Option E is a good complement** for GitHub-specific workflows that naturally involve PRs and CI, but doesn't generalize.

### Suggested Implementation Plan for Option B

1. **Define a "wait job" type** in the metis data model -- a lightweight job that runs a shell script instead of an LLM agent. It should support:
   - A command to run (shell string)
   - Expected output or exit code
   - Polling interval (default 30s)
   - Timeout (default 1h, max configurable)
   - Issue ID to close on success
   - Secret mounts for authentication

2. **Create a minimal container image** for wait jobs (~alpine + curl + gh CLI + metis CLI, <100MB).

3. **Add a CLI command** like `metis jobs wait` or extend issue creation to support wait conditions.

4. **Modify the spawner/job engine** to launch wait jobs as lightweight Kubernetes Jobs with minimal resource requests.

5. **Add timeout handling** -- if the timeout expires, mark the issue as failed with an informative error.

## Open Questions

1. Should the wait command syntax be shell-based (`--command "gh run view ..."`) or structured (`--type github-action --run-id 123`)? Shell-based is more flexible but less safe; structured is safer but requires predefined types.
2. Should wait jobs count against the agent's `max_simultaneous` capacity? Probably not, since they use minimal resources.
3. How should playbook authors discover the run ID or other identifiers to wait on? Should the system capture these from earlier steps automatically?
4. Is there a need for wait jobs to produce output (e.g., the final status of the GitHub Action) that downstream issues can consume?

## Research Sources

- [Claude Code session limits and 5-hour rolling window](https://claudelog.com/claude-code-limits/)
- [Claude Code headless/CLI mode documentation](https://code.claude.com/docs/en/headless)
- [GitHub Actions workflow_run event for chaining workflows](https://docs.github.com/actions/learn-github-actions/events-that-trigger-workflows)
- [GitHub Checks API for programmatic status checking](https://docs.github.com/en/rest/guides/using-the-rest-to-interact-with-checks)
- [Wait on Check GitHub Action for cross-workflow waiting](https://github.com/marketplace/actions/wait-on-check)
