#!/usr/bin/env bash
set -euo pipefail

WORKER_IMAGE="${WORKER_IMAGE:-hydra-worker:latest}"
SERVER_IMAGE="${SERVER_IMAGE:-hydra-server:latest}"
S3_IMAGE="${S3_IMAGE:-hydra-s3:latest}"
KIND_CLUSTER_NAME="${KIND_CLUSTER_NAME:-local-dev}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing dependency: $1. Please install it before running this script." >&2
    exit 1
  fi
}

require_cmd docker
require_cmd kind

if ! docker info >/dev/null 2>&1; then
  echo "Docker daemon is not running or not reachable. Start Docker and retry." >&2
  exit 1
fi

if ! kind get clusters | grep -qx "${KIND_CLUSTER_NAME}"; then
  echo "Kind cluster '${KIND_CLUSTER_NAME}' not found. Create it with:" >&2
  echo "  kind create cluster --name ${KIND_CLUSTER_NAME}" >&2
  exit 1
fi

docker build -t "${WORKER_IMAGE}" -f ./images/metis-worker.Dockerfile .
docker build -t "${SERVER_IMAGE}" -f ./images/metis-server.Dockerfile .
docker build -t "${S3_IMAGE}" -f ./images/metis-s3.Dockerfile .

kind load docker-image "${WORKER_IMAGE}" --name "${KIND_CLUSTER_NAME}"
kind load docker-image "${SERVER_IMAGE}" --name "${KIND_CLUSTER_NAME}"
kind load docker-image "${S3_IMAGE}" --name "${KIND_CLUSTER_NAME}"
