#!/usr/bin/env bash
#
# Initialize a metis server with the default set of agent prompts.
# Uploads prompt files from scripts/agent-prompts/ to the document store.
# Idempotent: creates documents on first run, updates only changed ones on subsequent runs.
#
# Usage:
#   ./scripts/init-agents.sh
#
# The script uses the METIS_SERVER_URL environment variable if set,
# which is passed through to the metis CLI automatically.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROMPTS_DIR="${SCRIPT_DIR}/agent-prompts"
SYNC_DIR="$(mktemp -d)"
trap 'rm -rf "$SYNC_DIR"' EXIT

AGENTS=("swe" "pm" "reviewer" "merger")

# Validate that all prompt files exist before syncing
for agent in "${AGENTS[@]}"; do
  if [[ ! -f "${PROMPTS_DIR}/${agent}.md" ]]; then
    echo "ERROR: Missing prompt file: ${PROMPTS_DIR}/${agent}.md" >&2
    exit 1
  fi
done

# Sync existing agent documents, copy in the latest prompts, and push
metis documents sync "$SYNC_DIR" --path-prefix /agents
for agent in "${AGENTS[@]}"; do
  mkdir -p "${SYNC_DIR}/agents/${agent}"
  cp "${PROMPTS_DIR}/${agent}.md" "${SYNC_DIR}/agents/${agent}/prompt.md"
done
metis documents push "$SYNC_DIR" --path-prefix /agents

echo "All agent prompts uploaded successfully."
