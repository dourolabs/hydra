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

  # clean git working tree
  git reset --hard
}

cleanup_tasks() {
  echo "Running cleanup tasks..."

  git diff > changes.patch

  cat output.txt
  cat changes.patch
}

startup_tasks

"$@"
cmd_status=$?

cleanup_tasks || true

exit $cmd_status
