#!/usr/bin/env bash
# E2E test runner for Hydra
#
# Bootstraps a fresh Hydra instance, registers the tester agent with
# Playwright MCP, creates a test issue, and monitors it to completion.
#
# Usage: ./tests/e2e/run.sh
#
# Required environment variables:
#   CLAUDE_CODE_OAUTH_TOKEN  OAuth token for Claude Code
#   GH_TOKEN                 GitHub personal access token (repo scope)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CONFIG_PATH="${SCRIPT_DIR}/config/test-config.yaml"
SERVER_URL="http://localhost:8080"
HYDRA_STATE_DIR="${HOME}/.hydra/server"
SERVER_PID=""

# --------------------------------------------------------------------------
# Cleanup
# --------------------------------------------------------------------------
cleanup() {
  echo "Cleaning up..."
  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
    echo "Server process ${SERVER_PID} stopped."
  fi
}
trap cleanup EXIT

# --------------------------------------------------------------------------
# 1. Validate prerequisites
# --------------------------------------------------------------------------
echo "==> Validating prerequisites..."

missing=()
if [[ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" ]]; then
  missing+=("CLAUDE_CODE_OAUTH_TOKEN")
fi
if [[ -z "${GH_TOKEN:-}" ]]; then
  missing+=("GH_TOKEN")
fi
if [[ ${#missing[@]} -gt 0 ]]; then
  echo "ERROR: Missing required environment variables: ${missing[*]}" >&2
  exit 1
fi

if ! command -v hydra &>/dev/null; then
  echo "ERROR: 'hydra' binary not found. Build it with: cargo build -p hydra" >&2
  exit 1
fi

if ! command -v npx &>/dev/null; then
  echo "ERROR: 'npx' not found. Install Node.js to get npx (needed for Playwright MCP)." >&2
  exit 1
fi

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "ERROR: Test config not found at ${CONFIG_PATH}" >&2
  exit 1
fi

echo "    All prerequisites met."

# --------------------------------------------------------------------------
# 2. Clean previous state
# --------------------------------------------------------------------------
if [[ -d "${HYDRA_STATE_DIR}" ]]; then
  echo "==> Removing previous server state at ${HYDRA_STATE_DIR}..."
  rm -rf "${HYDRA_STATE_DIR}"
fi

# --------------------------------------------------------------------------
# 3. Initialize and start server
# --------------------------------------------------------------------------
echo "==> Initializing server with test config..."
hydra server init --config "${CONFIG_PATH}"

echo "==> Starting server..."
hydra server run &
SERVER_PID=$!

echo "    Server PID: ${SERVER_PID}"
echo "==> Waiting for server health check..."

MAX_WAIT=30
WAITED=0
until curl -sf "${SERVER_URL}/health" >/dev/null 2>&1; do
  if [[ ${WAITED} -ge ${MAX_WAIT} ]]; then
    echo "ERROR: Server did not become healthy within ${MAX_WAIT}s" >&2
    exit 1
  fi
  sleep 1
  WAITED=$((WAITED + 1))
done
echo "    Server is healthy (waited ${WAITED}s)."

# --------------------------------------------------------------------------
# Read auth token for API calls
# --------------------------------------------------------------------------
AUTH_TOKEN_FILE="${HYDRA_STATE_DIR}/auth-token"
if [[ ! -f "${AUTH_TOKEN_FILE}" ]]; then
  echo "ERROR: Auth token file not found at ${AUTH_TOKEN_FILE}" >&2
  exit 1
fi
AUTH_TOKEN="$(cat "${AUTH_TOKEN_FILE}")"
export HYDRA_SERVER_URL="${SERVER_URL}"
export HYDRA_TOKEN="${AUTH_TOKEN}"

# --------------------------------------------------------------------------
# 4. Register tester agent
# --------------------------------------------------------------------------
echo "==> Uploading tester agent documents..."

# Create the MCP config document in the doc store
MCP_CONFIG='{"mcpServers":{"playwright":{"command":"npx","args":["@anthropic-ai/mcp-playwright"]}}}'
hydra documents create \
  --title "Tester MCP Config" \
  --path "/agents/tester/mcp-config.json" \
  --body "${MCP_CONFIG}"

# Write the tester prompt to a temp file for --prompt-file
TESTER_PROMPT_FILE="$(mktemp)"
cat > "${TESTER_PROMPT_FILE}" << 'PROMPT_EOF'
You are a tester agent responsible for running end-to-end test scenarios against the Hydra dashboard.
Your goal is to execute test scenarios using Playwright MCP, verify expected behavior, and report results.

Tools you can use:
- Issue tracker -- use the "hydra issues" command
- Todo list -- use the "hydra issues todo" command
- Pull requests -- use the "hydra patches" command (read-only for status)
- Documents -- use the "hydra documents" command
- Notifications -- use the "hydra notifications" command
- Playwright MCP -- browser automation tools for interacting with the dashboard

**Your issue id is stored in the HYDRA_ISSUE_ID environment variable.**

## Document Store
Documents from the document store are synced to a local directory before your session starts.
The path to this directory is available in the $HYDRA_DOCUMENTS_DIR environment variable.
Prefer reading and editing files in HYDRA_DOCUMENTS_DIR directly using standard filesystem tools.
The hydra documents CLI commands are available for operations that require server-side filtering
(e.g., listing by path prefix) but local filesystem access is preferred for reads and writes.
IMPORTANT: if you edit files in this directory, you must push them back to the central store
using `hydra documents push`.

Available CLI commands (use only when filesystem access is insufficient):
- `hydra documents list` -- list documents (supports --path-prefix for filtering)
- `hydra documents get <path>` -- get a specific document
- `hydra documents put <path> --file <file>` -- upload a document
- `hydra documents sync <directory>` -- sync documents to a local directory
- `hydra documents push <directory>` -- push local changes back to the store

## Playwright MCP Tools

You have access to the following Playwright MCP tools for browser automation:
- `browser_navigate` -- navigate to a URL
- `browser_screenshot` -- take a screenshot of the current page
- `browser_click` -- click an element on the page
- `browser_type` -- type text into an input field
- `browser_snapshot` -- take an accessibility snapshot of the page
- `browser_hover` -- hover over an element
- `browser_select_option` -- select an option from a dropdown
- `browser_wait_for_event` -- wait for a specific browser event
- `browser_tab_list` -- list open browser tabs
- `browser_tab_new` -- open a new browser tab
- `browser_tab_select` -- switch to a specific tab
- `browser_tab_close` -- close a browser tab

## Handling user feedback

After gathering context about the issue (via notifications or `hydra issues describe`), check the `feedback` field.
If the `feedback` field is populated, the user has submitted feedback on your prior work. You MUST:
1. Read the feedback carefully.
2. Acknowledge the feedback in the progress field.
3. Adjust your approach based on the feedback.
4. Address the feedback in your work.
5. Clear the feedback field when done:
   `hydra issues update $HYDRA_ISSUE_ID --feedback ""`

## Test Execution Workflow

1. **Check notifications**: Run `hydra notifications list --unread` to understand what changed since your last session.
   If there are no notifications (e.g., first invocation), fall back to: `hydra issues describe $HYDRA_ISSUE_ID`.

2. **Mark in-progress**: Update the issue status:
   `hydra issues update $HYDRA_ISSUE_ID --status in-progress --progress "Starting test execution"`

3. **Verify server health**: Before running any browser-based tests, confirm the Hydra server is running:
   - Run `curl -s http://localhost:8080/health` and verify a success response.
   - If the server is not running, report the failure in the progress field and set status to failed.

4. **Load test scenarios**: Read test scenarios from `tests/e2e/scenarios/` in the repository.
   - Scenarios are markdown files describing steps, expected results, and priority (P0/P1).
   - Execute all P0 scenarios first, then P1 scenarios.

5. **Execute each scenario**: For each scenario:
   a. Update progress with the scenario name: `hydra issues update $HYDRA_ISSUE_ID --progress "Running: <scenario-name>"`
   b. Follow the steps described in the scenario file.
   c. Use Playwright MCP tools to interact with the dashboard at `http://localhost:8080`.
   d. Take screenshots at key checkpoints using `browser_screenshot`.
   e. Use `browser_snapshot` to capture accessibility snapshots for verifying page structure.
   f. Compare actual results against the expected results in the scenario.

6. **Report results**: After all scenarios are complete, update the progress field with a summary:
   - Total scenarios run
   - Pass/fail count for each priority level (P0, P1)
   - Brief description of any failures, including screenshots

7. **Close the issue**: If all P0 scenarios pass, set the issue status to closed.
   If any P0 scenario fails, set the status to failed with details in the progress field.
   P1 failures should be noted but do not block closing.

## Test Execution Guidelines

- Always navigate to the dashboard URL (`http://localhost:8080`) before starting browser interactions.
- Take a screenshot after each major step for evidence and debugging.
- Use accessibility snapshots (`browser_snapshot`) to verify page structure rather than relying solely on visual checks.
- If a step fails, capture the error state (screenshot + snapshot) before moving to the next scenario.
- Wait for page loads and network requests to complete before interacting with elements.
- Report issues with specific details: what was expected, what actually happened, and relevant screenshots.

## Team Coordination

You are working on a team with multiple agents, any of which can pick up an issue to work on it. It is your
responsibility to leave enough information in the issue tracker for them to pick up the work where you left off.
Use the todo list, the progress field and the issue status to communicate this information with your team.
When you start working on the issue, you must set the status to in-progress.
When you finish working on the issue, you must set the status to closed.

hydra issues update $HYDRA_ISSUE_ID --progress <progress> --status <open|in-progress|closed|failed>
hydra issues todo $HYDRA_ISSUE_ID --add "thing that needs to be done"
hydra issues todo $HYDRA_ISSUE_ID --done 1

Before ending your session, mark all notifications as read: `hydra notifications read-all`
PROMPT_EOF

echo "==> Creating tester agent..."

# The CLI doesn't support --mcp-config-path, so we create the agent via CLI
# first, then update it via the REST API to set the mcp_config_path.
hydra agents create tester --prompt-file "${TESTER_PROMPT_FILE}"
rm -f "${TESTER_PROMPT_FILE}"

# Set mcp_config_path via the REST API (not yet exposed in the CLI)
echo "==> Setting MCP config path on tester agent..."
curl -sf -X PUT "${SERVER_URL}/v1/agents/tester" \
  -H "Authorization: Bearer ${AUTH_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "$(cat <<'API_EOF'
{"mcp_config_path": "/agents/tester/mcp-config.json"}
API_EOF
)" >/dev/null

echo "    Tester agent registered with MCP config."

# --------------------------------------------------------------------------
# 5. Add the test fixture repo
# --------------------------------------------------------------------------
echo "==> Adding test fixture repository..."
hydra repos create dourolabs/hydra-test-fixture \
  "https://github.com/dourolabs/hydra-test-fixture.git"

# --------------------------------------------------------------------------
# 6. Create test issue
# --------------------------------------------------------------------------
echo "==> Creating test issue for tester agent..."
ISSUE_OUTPUT="$(hydra issues create \
  --title "Run E2E test scenarios" \
  --assignee tester \
  --output-format jsonl \
  "Execute all P0 and P1 test scenarios from tests/e2e/scenarios/. Report results in the progress field.")"

ISSUE_ID="$(echo "${ISSUE_OUTPUT}" | python3 -c "import sys,json; print(json.load(sys.stdin)['issue_id'])" 2>/dev/null || echo "${ISSUE_OUTPUT}" | sed -n 's/.*"issue_id":"\([^"]*\)".*/\1/p')"

if [[ -z "${ISSUE_ID}" ]]; then
  echo "ERROR: Failed to extract issue ID from create output" >&2
  echo "Output was: ${ISSUE_OUTPUT}" >&2
  exit 1
fi

echo "    Created issue: ${ISSUE_ID}"

# --------------------------------------------------------------------------
# 7. Monitor and report
# --------------------------------------------------------------------------
echo "==> Monitoring issue ${ISSUE_ID} until completion..."
POLL_INTERVAL=15

while true; do
  ISSUE_JSON="$(hydra issues get "${ISSUE_ID}" --output-format jsonl 2>/dev/null || true)"

  if [[ -z "${ISSUE_JSON}" ]]; then
    echo "    Warning: failed to fetch issue status, retrying..."
    sleep "${POLL_INTERVAL}"
    continue
  fi

  STATUS="$(echo "${ISSUE_JSON}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('issue',{}).get('status','unknown'))" 2>/dev/null || echo "unknown")"
  PROGRESS="$(echo "${ISSUE_JSON}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('issue',{}).get('progress',''))" 2>/dev/null || echo "")"

  echo "    Status: ${STATUS} | Progress: ${PROGRESS}"

  case "${STATUS}" in
    closed)
      echo ""
      echo "=========================================="
      echo "  E2E TESTS PASSED"
      echo "=========================================="
      echo ""
      echo "Results: ${PROGRESS}"
      exit 0
      ;;
    failed)
      echo ""
      echo "=========================================="
      echo "  E2E TESTS FAILED"
      echo "=========================================="
      echo ""
      echo "Results: ${PROGRESS}"
      exit 1
      ;;
    *)
      sleep "${POLL_INTERVAL}"
      ;;
  esac
done
