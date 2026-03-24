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
#   CLAUDE_CODE_OAUTH_TOKEN  OAuth token for Claude Code
#   GH_TOKEN                 GitHub personal access token (repo scope)
#
# The script leaves the server running in the background. Use the printed
# PID or the cleanup trap (on script exit) to stop it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CONFIG_PATH="${SCRIPT_DIR}/config/test-config.yaml"
SERVER_URL="http://localhost:8080"
HYDRA_STATE_DIR="${HOME}/.hydra/server"
HYDRA_BIN="${REPO_ROOT}/target/release/hydra"
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
echo "    Binary: ${HYDRA_BIN}"

# --------------------------------------------------------------------------
# 5. Initialize and start server
# --------------------------------------------------------------------------
echo "==> Initializing server with test config..."
"${HYDRA_BIN}" server init --config "${CONFIG_PATH}"

echo "==> Starting server..."
"${HYDRA_BIN}" server start &
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
# 6. Pre-register test fixture repository
# --------------------------------------------------------------------------
echo "==> Registering dourolabs/hydra-test-fixture repository..."
"${HYDRA_BIN}" repos create dourolabs/hydra-test-fixture https://github.com/dourolabs/hydra-test-fixture.git
echo "    Repository registered."

# --------------------------------------------------------------------------
# Print connection info
# --------------------------------------------------------------------------
echo ""
echo "=========================================="
echo "  Hydra test server is running"
echo "=========================================="
echo ""
echo "  URL:  ${SERVER_URL}"
echo "  PID:  ${SERVER_PID}"
echo ""
echo "The server will be stopped when this script exits."
echo "Press Ctrl+C to stop, or use 'kill ${SERVER_PID}' from another terminal."
echo ""

# Keep the script running so the trap can clean up on Ctrl+C
wait "${SERVER_PID}"
