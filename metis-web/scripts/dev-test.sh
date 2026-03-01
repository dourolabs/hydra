#!/bin/bash
# metis-web/scripts/dev-test.sh
# Start mock server + BFF + Vite dev server, then optionally run E2E tests.
#
# Usage:
#   ./scripts/dev-test.sh          # Start dev stack and keep running
#   ./scripts/dev-test.sh --test   # Start dev stack, run E2E tests, then exit
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

MOCK_PID=""
BFF_PID=""
VITE_PID=""

cleanup() {
  echo ""
  echo "Shutting down dev stack..."
  [ -n "$VITE_PID" ] && kill "$VITE_PID" 2>/dev/null || true
  [ -n "$BFF_PID" ] && kill "$BFF_PID" 2>/dev/null || true
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

wait_for_port() {
  local port=$1
  local name=$2
  local timeout=${3:-30}
  local elapsed=0
  echo "Waiting for $name on port $port..."
  while ! curl -sf "http://localhost:$port" >/dev/null 2>&1; do
    sleep 1
    elapsed=$((elapsed + 1))
    if [ "$elapsed" -ge "$timeout" ]; then
      echo "ERROR: $name failed to start on port $port within ${timeout}s"
      exit 1
    fi
  done
  echo "$name is ready on port $port"
}

wait_for_url() {
  local url=$1
  local name=$2
  local timeout=${3:-30}
  local elapsed=0
  echo "Waiting for $name at $url..."
  while ! curl -sf "$url" >/dev/null 2>&1; do
    sleep 1
    elapsed=$((elapsed + 1))
    if [ "$elapsed" -ge "$timeout" ]; then
      echo "ERROR: $name failed to respond at $url within ${timeout}s"
      exit 1
    fi
  done
  echo "$name is ready"
}

# Start mock server (port 8080)
echo "Starting mock server..."
pnpm --filter @metis/mock-server dev &
MOCK_PID=$!
wait_for_url "http://localhost:8080/health" "Mock server" 30

# Start BFF (port 4000), pointing at mock server
echo "Starting BFF server..."
METIS_SERVER_URL=http://localhost:8080 COOKIE_SECURE=false pnpm --filter @metis/web dev:server &
BFF_PID=$!
wait_for_url "http://localhost:4000/health" "BFF server" 30

# Build API and UI packages, then start Vite dev server (port 3000)
echo "Building API and UI packages..."
pnpm --filter @metis/api build && pnpm --filter @metis/ui build
echo "Starting Vite dev server..."
pnpm --filter @metis/web dev &
VITE_PID=$!
wait_for_port 3000 "Vite dev server" 60

echo ""
echo "========================================="
echo "  Dev stack ready!"
echo "========================================="
echo "  Mock server: http://localhost:8080"
echo "  BFF:         http://localhost:4000"
echo "  Frontend:    http://localhost:3000"
echo "========================================="
echo ""

if [[ "${1:-}" == "--test" ]]; then
  echo "Running Playwright E2E tests..."
  pnpm --filter @metis/web exec playwright test
  exit $?
fi

# Keep running until Ctrl-C
wait
