# Hydra E2E Testing

End-to-end test scenarios for Hydra. A testing agent executes these scenarios via Playwright MCP against the Hydra dashboard running in single-player mode.

## Overview

Each scenario describes a sequence of **frontend dashboard interactions** that validate Hydra's functionality. Scenarios are written as markdown files and executed by a testing agent using Playwright MCP for headless browser automation.

All test interactions go through the dashboard UI at `http://localhost:8080` -- scenarios do not use CLI commands for testing.

## Directory Structure

```
tests/e2e/
├── README.md              # This file
├── run.sh                 # E2E test runner script
├── config/
│   ├── test-config.yaml   # Server init config for test runs
│   └── merge-policy.yaml  # Merge policy applied to the test-fixture repo
└── scenarios/
    ├── server-init.md           # P0: Server initialization and dashboard load
    ├── add-github-repo.md       # P0: Verify pre-registered GitHub repo
    ├── basic-issue-lifecycle.md  # P0: Issue creation through closure
    ├── dashboard-navigation.md   # P0: Verify all dashboard pages load
    ├── pm-agent-breakdown.md     # P1: PM agent decomposes a high-level issue
    ├── reviewer-agent.md         # P1: Reviewer agent reviews a patch
    └── agent-handoff.md          # P1: Full PM -> SWE -> Reviewer flow
```

## Scenario Format

Each scenario file follows this structure:

```markdown
# Scenario: <Title>

**ID:** <kebab-case-id>
**Category:** <core | agent-coordination>
**Priority:** <P0 | P1>
**Prerequisites:** <What must be true before running this scenario>
**Estimated duration:** <Expected time to complete>

## Description

<What the scenario tests and why it matters.>

## Steps (via dashboard)

<Numbered list of dashboard UI interactions using Playwright MCP:
navigate to pages, click buttons, fill forms, wait for state changes.>

## Expected Results

<What the tester should see in the dashboard after executing all steps.>
```

## Priority Levels

- **P0 (Core):** Must pass for every release. Covers server init, basic repo management, issue lifecycle, and dashboard rendering.
- **P1 (Agent Coordination):** Validates multi-agent workflows including PM task breakdown, code review, and full agent handoff.

## Adding New Scenarios

1. Create a new markdown file in `tests/e2e/scenarios/` following the format above.
2. Use a descriptive kebab-case filename (e.g., `my-new-scenario.md`).
3. Assign an appropriate priority (P0 for core functionality, P1 for agent coordination, P2+ for edge cases).
4. Ensure all steps describe dashboard UI interactions, not CLI commands.
5. List prerequisites so the testing agent knows which scenarios must pass first.

## Test Environment

| Requirement | Details |
|---|---|
| Hydra binary | Built from source (`cargo build -p hydra-single-player --release`) |
| Claude credential | Either `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` environment variable |
| GitHub PAT | `GH_TOKEN` environment variable |
| Playwright MCP | `npx @playwright/mcp` |
| Test repo | `dourolabs/hydra-test-fixture` |

## Running Tests

### Quick Start

```bash
# Set required environment variables.
# Provide at least one Claude credential (either OAuth token or API key):
export CLAUDE_CODE_OAUTH_TOKEN="your-oauth-token"
# ...or:
# export ANTHROPIC_API_KEY="your-anthropic-api-key"
export GH_TOKEN="your-github-pat"

# Bootstrap a test server. Returns immediately after the server is healthy;
# the server keeps running detached in the background.
./tests/e2e/run.sh

# Stop the server when you're done.
kill "$(cat /tmp/hydra-e2e/server.pid)"
```

### What the Runner Does

The `run.sh` script is a lightweight utility that bootstraps a fresh Hydra single-player instance for testing:

1. **Validates prerequisites** -- checks for required env vars (`GH_TOKEN` plus at least one of `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY`), `cargo`, `npx`, and the test config file
2. **Creates directories** -- sets up test paths (`/tmp/hydra-e2e`)
3. **Cleans previous state** -- removes `~/.hydra/server/` for a fresh run
4. **Builds the binary** -- runs `cargo build -p hydra-single-player --release` and creates a `hydra-sp` symlink in `target/release/`
5. **Initializes the server** -- runs `hydra-sp server init` with the test config
6. **Starts the server** -- runs `hydra-sp server start` in the background and waits for the health check at `http://localhost:8080/health`
7. **Registers test fixture repo** -- runs `hydra-sp repos create` with explicit `HYDRA_SERVER_URL` to pre-register `dourolabs/hydra-test-fixture`
8. **Applies a merge policy** -- runs `hydra-sp repos update dourolabs/hydra-test-fixture --merge-policy-file tests/e2e/config/merge-policy.yaml` so e2e scenarios exercise the real merge-time-constraints workflow (required `reviewer` approval; anyone may merge)

The `hydra-sp` symlink points to the `hydra` binary and exists to avoid conflicting with a production `hydra` CLI when testing Hydra-in-Hydra. The `HYDRA_SERVER_URL` env var is set explicitly on repo registration to target the local test instance.

On success, the script exits with status 0 and leaves the server running detached in the background. It writes the server PID to `/tmp/hydra-e2e/server.pid`; the caller is responsible for stopping the server when done (e.g., `kill "$(cat /tmp/hydra-e2e/server.pid)"`). If bootstrap fails (health-check timeout, repo-create error, etc.), the script kills any partially-started server and exits non-zero.

The tester agent (running in the top-level Hydra instance) is responsible for executing
test scenarios against the server using Playwright MCP. The tester agent's prompt and MCP
config live in the top-level Hydra instance's doc store.

### Scenario Execution

Scenarios are executed by the tester agent against the server that `run.sh` has already started. The agent:

1. Reads scenario files from this directory
2. Executes each scenario's steps via the dashboard using Playwright MCP
3. Verifies expected results
4. Reports pass/fail status

See the design document at `/designs/hydra-e2e-testing-process.md` in the document store for the full architecture.
