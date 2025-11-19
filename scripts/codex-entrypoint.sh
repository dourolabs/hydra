#!/usr/bin/env bash

set -euo pipefail

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

  metis context $METIS_ID . 
}

cleanup_tasks() {
  echo "Running cleanup tasks..."

  git diff --cached > changes.patch

  echo "Uploading job output via Metis CLI..."
  if ! metis set-output "${METIS_ID}" --last-message output.txt --patch changes.patch; then
    echo "Failed to set job output via Metis CLI." >&2
  else
    echo "Job output uploaded successfully."
  fi
}

startup_tasks

"$@"
cmd_status=$?

cleanup_tasks || true

exit $cmd_status
