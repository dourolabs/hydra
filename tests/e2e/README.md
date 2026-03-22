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
│   └── test-config.yaml   # Server init config for test runs
└── scenarios/
    ├── server-init.md           # P0: Server initialization and dashboard load
    ├── add-github-repo.md       # P0: Add a GitHub repo via dashboard issue
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
| Hydra binary | Built from source (`cargo build -p hydra`) |
| API key | `CLAUDE_CODE_OAUTH_TOKEN` environment variable |
| GitHub PAT | `GH_TOKEN` environment variable |
| Playwright MCP | `npx @anthropic-ai/mcp-playwright` |
| Test repo | `dourolabs/hydra-test-fixture` |

## Running Tests

### Quick Start

```bash
# Set required environment variables
export CLAUDE_CODE_OAUTH_TOKEN="your-oauth-token"
export GH_TOKEN="your-github-pat"

# Bootstrap a test server
./tests/e2e/run.sh
```

### What the Runner Does

The `run.sh` script is a lightweight utility that bootstraps a fresh Hydra instance for testing:

1. **Validates prerequisites** -- checks for required env vars, the `hydra` binary, and `npx`
2. **Cleans previous state** -- removes `~/.hydra/server/` for a fresh run
3. **Starts the server** -- runs `hydra server init` with the test config, starts `hydra server run`, and waits for the health check

The script keeps the server running in the foreground and cleans up on exit (Ctrl+C).

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
