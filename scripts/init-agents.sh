#!/usr/bin/env bash
#
# Initialize a metis server with the default set of agent prompts.
# Uploads prompt files from scripts/agent-prompts/ to the document store.
# Idempotent: creates documents on first run, updates them on subsequent runs.
#
# Usage:
#   ./scripts/init-agents.sh
#
# The script uses the METIS_SERVER_URL environment variable if set,
# which is passed through to the metis CLI automatically.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROMPTS_DIR="${SCRIPT_DIR}/agent-prompts"

AGENTS=("swe" "pm" "reviewer" "merger")

for agent in "${AGENTS[@]}"; do
  prompt_file="${PROMPTS_DIR}/${agent}.md"
  doc_path="/agents/${agent}/prompt.md"
  title="${agent^} Agent Prompt"

  if [[ ! -f "$prompt_file" ]]; then
    echo "ERROR: Missing prompt file: ${prompt_file}" >&2
    exit 1
  fi

  # Check if the document already exists at this path
  existing_id=$(metis documents get "$doc_path" --output-format jsonl 2>/dev/null \
    | head -1 | sed -n 's/.*"document_id":"\([^"]*\)".*/\1/p') || true

  if [[ -n "$existing_id" ]]; then
    echo "Updating ${agent} prompt (${existing_id})..."
    # The CLI returns an error when the body is unchanged; treat that as already up-to-date
    output=$(metis documents update "$existing_id" --body-file "$prompt_file" 2>&1) || {
      if echo "$output" | grep -q "no updates specified"; then
        echo "  (already up-to-date)"
      else
        echo "$output" >&2
        exit 1
      fi
    }
  else
    echo "Creating ${agent} prompt..."
    metis documents create --title "$title" --path "$doc_path" --body-file "$prompt_file"
  fi
done

echo "All agent prompts uploaded successfully."
