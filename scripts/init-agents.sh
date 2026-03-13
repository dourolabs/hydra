#!/usr/bin/env bash
#
# Initialize a metis server with the default set of agents.
# Creates agents in the database and uploads their prompts to the document store
# via CLI commands. Idempotent: creates on first run, updates on subsequent runs.
#
# Usage:
#   ./scripts/init-agents.sh
#
# The script uses the METIS_SERVER_URL environment variable if set,
# which is passed through to the metis CLI automatically.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROMPTS_DIR="${REPO_ROOT}/prompts/agents"

AGENTS=("swe" "pm" "reviewer")

# Validate that all prompt files exist before creating agents
for agent in "${AGENTS[@]}"; do
  if [[ ! -f "${PROMPTS_DIR}/${agent}.md" ]]; then
    echo "ERROR: Missing prompt file: ${PROMPTS_DIR}/${agent}.md" >&2
    exit 1
  fi
done

# Create or update each agent via the CLI
for agent in "${AGENTS[@]}"; do
  EXTRA_FLAGS=()
  if [[ "$agent" == "pm" ]]; then
    EXTRA_FLAGS+=("--is-assignment-agent")
  fi

  create_args=("$agent" "--prompt-file" "${PROMPTS_DIR}/${agent}.md")
  (( ${#EXTRA_FLAGS[@]} > 0 )) && create_args+=("${EXTRA_FLAGS[@]}")

  if metis agents create "${create_args[@]}" 2>/dev/null; then
    echo "Created agent: ${agent}"
  else
    update_args=("$agent" "--prompt-file" "${PROMPTS_DIR}/${agent}.md")
    (( ${#EXTRA_FLAGS[@]} > 0 )) && update_args+=("${EXTRA_FLAGS[@]}")
    metis agents update "${update_args[@]}"
    echo "Updated agent: ${agent}"
  fi
done

echo "All agents initialized successfully."
