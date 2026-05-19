#!/usr/bin/env bash
# E2E server bootstrap utility for Hydra
#
# Bootstraps a fresh Hydra single-player instance for E2E testing.
# The tester agent (running in the top-level Hydra instance) uses this
# script to get a test server running, then executes test scenarios
# against it via Playwright MCP.
#
# Usage: ./tests/e2e/run.sh
#
# Required environment variables:
#   CLAUDE_CODE_OAUTH_TOKEN  OAuth token for Claude Code (or CLAUDE_TEST_TOKEN / ANTHROPIC_API_KEY)
#   CLAUDE_TEST_TOKEN        Fallback name for the Claude OAuth token (used by the e2e tester agent)
#   ANTHROPIC_API_KEY        Anthropic API key (alternative to the OAuth token)
#   GH_TOKEN                 GitHub personal access token (repo scope)
#
# At least one of CLAUDE_CODE_OAUTH_TOKEN, CLAUDE_TEST_TOKEN, or ANTHROPIC_API_KEY must be set.
# CLAUDE_CODE_OAUTH_TOKEN is preferred; CLAUDE_TEST_TOKEN is accepted as a fallback and
# promoted to CLAUDE_CODE_OAUTH_TOKEN for downstream consumers.
#
# On success, the script exits with status 0 and leaves the server running
# detached in the background. The server PID is written to
# /tmp/hydra-e2e/server.pid; the caller is responsible for stopping it
# (e.g., `kill "$(cat /tmp/hydra-e2e/server.pid)"`).
#
# On failure (bootstrap error, health-check timeout, repo-create failure),
# the script kills any partially-started server and exits non-zero.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CONFIG_PATH="${SCRIPT_DIR}/config/test-config.yaml"
# MUST match `server_hostname` in test-config.yaml so the CLI's saved-token lookup hits.
SERVER_URL="http://127.0.0.1:8080"
HYDRA_STATE_DIR="${HOME}/.hydra/server"
HYDRA_SERVER_PID_FILE="${HYDRA_STATE_DIR}/hydra-server.pid"
HYDRA_SP="${REPO_ROOT}/target/release/hydra-sp"
PID_FILE="/tmp/hydra-e2e/server.pid"
SERVER_PID=""
SUCCESS=0

# --------------------------------------------------------------------------
# Cleanup — only kills the server if bootstrap failed. On success we leave
# it running detached so the caller can drive scenarios against it.
# --------------------------------------------------------------------------
cleanup() {
  if [[ ${SUCCESS} -eq 1 ]]; then
    return
  fi
  if [[ -n "${SERVER_PID}" ]]; then
    echo "Cleaning up failed bootstrap..."
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

# Accept either CLAUDE_CODE_OAUTH_TOKEN (preferred) or CLAUDE_TEST_TOKEN (fallback,
# used by the e2e tester agent), promoting the fallback so downstream consumers
# only need to look at CLAUDE_CODE_OAUTH_TOKEN.
if [[ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" && -n "${CLAUDE_TEST_TOKEN:-}" ]]; then
  export CLAUDE_CODE_OAUTH_TOKEN="${CLAUDE_TEST_TOKEN}"
fi

if [[ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" && -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "ERROR: At least one of CLAUDE_CODE_OAUTH_TOKEN, CLAUDE_TEST_TOKEN, or ANTHROPIC_API_KEY must be set" >&2
  exit 1
fi
if [[ -z "${GH_TOKEN:-}" ]]; then
  echo "ERROR: Missing required environment variable: GH_TOKEN" >&2
  exit 1
fi

if ! command -v cargo &>/dev/null; then
  echo "ERROR: 'cargo' not found. Install Rust to build hydra-single-player." >&2
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
# 2. Create directories for test-specific paths
# --------------------------------------------------------------------------
echo "==> Creating directories for test paths..."
mkdir -p /tmp/hydra-e2e

# --------------------------------------------------------------------------
# 3. Clean previous state
# --------------------------------------------------------------------------
if [[ -d "${HYDRA_STATE_DIR}" ]]; then
  echo "==> Removing previous server state at ${HYDRA_STATE_DIR}..."
  rm -rf "${HYDRA_STATE_DIR}"
fi

# --------------------------------------------------------------------------
# 4. Build hydra-single-player
# --------------------------------------------------------------------------
echo "==> Building hydra-single-player (release)..."
(cd "${REPO_ROOT}" && cargo build -p hydra-single-player --release)
ln -sf hydra "${REPO_ROOT}/target/release/hydra-sp"
echo "    Binary: ${HYDRA_SP}"

# --------------------------------------------------------------------------
# 5. Initialize server (which also starts it in-process if not already running)
# --------------------------------------------------------------------------
echo "==> Initializing server with test config..."
"${HYDRA_SP}" server init --config "${CONFIG_PATH}"

# `server init` starts the server itself; capture its PID from the server's own
# pidfile so the cleanup trap and the exported PID file refer to the real
# process (not the short-lived `init`/`start` CLI invocation).
if [[ ! -f "${HYDRA_SERVER_PID_FILE}" ]]; then
  echo "ERROR: Server PID file not found at ${HYDRA_SERVER_PID_FILE} after init" >&2
  exit 1
fi
SERVER_PID="$(cat "${HYDRA_SERVER_PID_FILE}")"
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
# 6. Pre-register test fixture repository
#
# `env -u HYDRA_TOKEN` drops any inherited token from the caller's shell (the
# tester agent exports one for talking to production); without it the local
# CLI would send the wrong token and the local server returns 401. With it
# unset, the CLI falls back to the auth-token file written by `server init`.
# --------------------------------------------------------------------------
echo "==> Registering dourolabs/hydra-test-fixture repository..."
env -u HYDRA_TOKEN HYDRA_SERVER_URL="${SERVER_URL}" "${HYDRA_SP}" repos create dourolabs/hydra-test-fixture https://github.com/dourolabs/hydra-test-fixture.git
echo "    Repository registered."

# --------------------------------------------------------------------------
# Detach the server and exit cleanly. The server keeps running in the
# background after this script returns; the caller stops it via the PID
# file.
# --------------------------------------------------------------------------
echo "${SERVER_PID}" > "${PID_FILE}"
disown "${SERVER_PID}" 2>/dev/null || true

echo ""
echo "=========================================="
echo "  Hydra test server is running"
echo "=========================================="
echo ""
echo "  URL:       ${SERVER_URL}"
echo "  PID:       ${SERVER_PID}"
echo "  PID file:  ${PID_FILE}"
echo ""
echo "The server is detached and will keep running after this script exits."
echo "Stop it with: kill \"\$(cat ${PID_FILE})\""
echo ""

SUCCESS=1
exit 0
