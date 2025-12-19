#!/usr/bin/env bash

set -euo pipefail

if [[ -n "${NVM_DIR:-}" && -s "$NVM_DIR/nvm.sh" ]]; then
  # shellcheck disable=SC1090
  source "$NVM_DIR/nvm.sh"
fi

# Run worker-run (includes codex login, output directory creation, and job submission)
metis worker-run "${METIS_ID}" .
