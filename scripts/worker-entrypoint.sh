#!/usr/bin/env bash

set -euo pipefail

if [[ -n "${NVM_DIR:-}" && -s "$NVM_DIR/nvm.sh" ]]; then
  # shellcheck disable=SC1090
  source "$NVM_DIR/nvm.sh"
fi

# Run worker-init (includes codex login and output directory creation)
metis worker-init "${METIS_ID}" .

# Run the main task (eg codex)
"$@"
cmd_status=$?

# Run worker-submit (includes git staging, patch creation, and upload)
metis worker-submit "${METIS_ID}" || true

exit $cmd_status
