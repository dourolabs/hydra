#!/usr/bin/env bash

set -euo pipefail

METIS_DIR=".metis"
OUTPUT_DIR="${METIS_DIR}/output"
OUTPUT_FILE="${OUTPUT_DIR}/output.txt"
PATCH_FILE="${OUTPUT_DIR}/changes.patch"

if [[ -n "${NVM_DIR:-}" && -s "$NVM_DIR/nvm.sh" ]]; then
  # shellcheck disable=SC1090
  source "$NVM_DIR/nvm.sh"
fi

startup_tasks() {
  echo "Running startup tasks..."

  # log in to codex
  if [[ -z "${OPENAI_API_KEY:-}" ]]; then
    echo "OPENAI_API_KEY is not set; unable to login Codex CLI." >&2
    exit 1
  fi
  printenv OPENAI_API_KEY | codex login --with-api-key

  metis worker-init "${METIS_ID}" .
  mkdir -p "${OUTPUT_DIR}"
}

cleanup_tasks() {
  echo "Running cleanup tasks..."

  # Exclude codex-generated output from the staged patch
  git add -A -- . ':!.metis/**'
  git diff --cached -- . ':!.metis/**' > "${PATCH_FILE}"

  echo "Uploading job output via Metis CLI..."
  if ! metis worker-submit "${METIS_ID}"; then
    echo "Failed to set job output via Metis CLI." >&2
  else
    echo "Job output uploaded successfully."
  fi
}

startup_tasks

# Run the main task (eg codex)
"$@"
cmd_status=$?

cleanup_tasks || true

exit $cmd_status
