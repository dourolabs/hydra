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

  local job_id="${METIS_ID:-}"
  if [[ -z "${job_id}" ]]; then
    echo "METIS_ID not set; skipping job output upload." >&2
    return
  fi

  if ! command -v node >/dev/null 2>&1; then
    echo "Node.js is unavailable; skipping job output upload." >&2
    return
  fi

  local server_host="${SERVER_SERVICE_HOST:-localhost}"
  local server_port="${SERVER_SERVICE_PORT:-8080}"
  local base_url="${METIS_SERVER_URL:-http://${server_host}:${server_port}}"
  base_url="${base_url%/}"
  local endpoint="${base_url}/v1/jobs/${job_id}/output"

  echo "Uploading job output to ${endpoint}..."

  local payload
  if ! payload="$(
    node <<'NODE'
const fs = require('fs');
const safeRead = (path) => {
  try {
    if (fs.existsSync(path)) {
      return fs.readFileSync(path, 'utf8');
    }
  } catch (err) {
    console.error(`Warning: failed to read ${path}: ${err.message}`);
  }
  return '';
};
const payload = {
  last_message: safeRead('output.txt'),
  patch: safeRead('changes.patch'),
};
process.stdout.write(JSON.stringify(payload));
NODE
  )"; then
    echo "Failed to serialize job output payload; skipping upload." >&2
    return
  fi

  if ! curl -fsS -X POST \
    -H "Content-Type: application/json" \
    --data "${payload}" \
    "${endpoint}"
  then
    echo "Failed to upload job output to metis-server." >&2
  else
    echo "Job output uploaded successfully."
  fi
}

startup_tasks

"$@"
cmd_status=$?

cleanup_tasks || true

exit $cmd_status
