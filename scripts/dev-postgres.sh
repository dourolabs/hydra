#!/usr/bin/env bash
set -euo pipefail

COMMAND="${1:-start}"

POSTGRES_IMAGE="${POSTGRES_IMAGE:-postgres:16-alpine}"
POSTGRES_CONTAINER_NAME="${POSTGRES_CONTAINER_NAME:-hydra-postgres}"
POSTGRES_PORT="${POSTGRES_PORT:-5432}"
POSTGRES_DB="${POSTGRES_DB:-hydra}"
POSTGRES_USER="${POSTGRES_USER:-hydra}"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-hydra}"
POSTGRES_VOLUME="${POSTGRES_VOLUME:-hydra-postgres-data}"

connection_url="postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@localhost:${POSTGRES_PORT}/${POSTGRES_DB}"

usage() {
  cat <<EOF
Usage: $0 [start|stop|status|destroy]

Environment overrides:
  POSTGRES_IMAGE            Image to run (default: ${POSTGRES_IMAGE})
  POSTGRES_CONTAINER_NAME   Container name (default: ${POSTGRES_CONTAINER_NAME})
  POSTGRES_PORT             Host port to expose (default: ${POSTGRES_PORT})
  POSTGRES_DB               Database name (default: ${POSTGRES_DB})
  POSTGRES_USER             Database user (default: ${POSTGRES_USER})
  POSTGRES_PASSWORD         Database password (default: ${POSTGRES_PASSWORD})
  POSTGRES_VOLUME           Docker volume for data (default: ${POSTGRES_VOLUME})
  REMOVE_VOLUME=1           Remove the Docker volume on destroy
EOF
}

require_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker is required but was not found in PATH." >&2
    exit 1
  fi
}

container_exists() {
  docker ps -a --format '{{.Names}}' | grep -Fxq "${POSTGRES_CONTAINER_NAME}"
}

container_running() {
  docker ps --format '{{.Names}}' | grep -Fxq "${POSTGRES_CONTAINER_NAME}"
}

start_container() {
  if container_running; then
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' is already running."
    return
  fi

  if container_exists; then
    echo "Starting existing Postgres container '${POSTGRES_CONTAINER_NAME}'..."
    docker start "${POSTGRES_CONTAINER_NAME}" >/dev/null
  else
    echo "Creating data volume '${POSTGRES_VOLUME}' (if needed)..."
    docker volume inspect "${POSTGRES_VOLUME}" >/dev/null 2>&1 || docker volume create "${POSTGRES_VOLUME}" >/dev/null

    echo "Launching Postgres container '${POSTGRES_CONTAINER_NAME}' on port ${POSTGRES_PORT}..."
    docker run -d \
      --name "${POSTGRES_CONTAINER_NAME}" \
      -p "${POSTGRES_PORT}:5432" \
      --restart unless-stopped \
      -e POSTGRES_DB="${POSTGRES_DB}" \
      -e POSTGRES_USER="${POSTGRES_USER}" \
      -e POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
      -v "${POSTGRES_VOLUME}:/var/lib/postgresql/data" \
      "${POSTGRES_IMAGE}" >/dev/null
  fi

  echo "Postgres is running on localhost:${POSTGRES_PORT}"
  echo "Connection string: ${connection_url}"
}

stop_container() {
  if container_running; then
    docker stop "${POSTGRES_CONTAINER_NAME}" >/dev/null
    echo "Postgres container stopped."
  else
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' is not running."
  fi
}

destroy_container() {
  if container_exists; then
    docker rm -f "${POSTGRES_CONTAINER_NAME}" >/dev/null
    echo "Removed Postgres container '${POSTGRES_CONTAINER_NAME}'."
  else
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' does not exist."
  fi

  if [[ "${REMOVE_VOLUME:-0}" == "1" ]]; then
    if docker volume inspect "${POSTGRES_VOLUME}" >/dev/null 2>&1; then
      docker volume rm -f "${POSTGRES_VOLUME}" >/dev/null
      echo "Removed data volume '${POSTGRES_VOLUME}'."
    else
      echo "Data volume '${POSTGRES_VOLUME}' does not exist."
    fi
  else
    echo "Data volume '${POSTGRES_VOLUME}' preserved (set REMOVE_VOLUME=1 to delete)."
  fi
}

print_status() {
  if container_running; then
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' is running."
    docker ps --filter "name=${POSTGRES_CONTAINER_NAME}" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
    echo "Connection string: ${connection_url}"
  elif container_exists; then
    status="$(docker ps -a --filter "name=${POSTGRES_CONTAINER_NAME}" --format "{{.Status}}")"
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' exists but is not running (status: ${status})."
    echo "Run '$0 start' to start it."
  else
    echo "Postgres container '${POSTGRES_CONTAINER_NAME}' has not been created yet."
  fi
}

require_docker

case "${COMMAND}" in
  start)
    start_container
    ;;
  stop)
    stop_container
    ;;
  status)
    print_status
    ;;
  destroy)
    destroy_container
    ;;
  *)
    usage
    exit 1
    ;;
esac
