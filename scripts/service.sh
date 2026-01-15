#!/usr/bin/env bash
set -euo pipefail

COMMAND="${1:-start}"

# -------- Configurable settings --------
# Override any of these by exporting them before running the script, e.g.:
#   NAMESPACE=my-app ./setup-server-client.sh

NAMESPACE="${NAMESPACE:-metis}"
SERVER_IMAGE="${SERVER_IMAGE:-metis-server:latest}"
CLIENT_IMAGE="${CLIENT_IMAGE:-metis-worker:latest}"
SERVER_REPLICAS="${SERVER_REPLICAS:-1}"

# Service type for external access:
# - LoadBalancer (default) for managed clusters (GKE/EKS/AKS, etc.)
# - NodePort for bare metal / kind / minikube
SERVER_SERVICE_TYPE="${SERVER_SERVICE_TYPE:-LoadBalancer}"
SERVER_CONFIGMAP_NAME="${SERVER_CONFIGMAP_NAME:-metis-server-config}"
SERVER_CONFIG_MOUNT_PATH="${SERVER_CONFIG_MOUNT_PATH:-/etc/metis}"
SERVER_CONFIG_FILE_NAME="${SERVER_CONFIG_FILE_NAME:-config.toml}"
SERVER_METIS_CONFIG_PATH="${SERVER_METIS_CONFIG_PATH:-${SERVER_CONFIG_MOUNT_PATH}/${SERVER_CONFIG_FILE_NAME}}"

# Config generation defaults (can be overridden by env vars)
SERVER_OPENAI_API_KEY="${SERVER_OPENAI_API_KEY:-${OPENAI_API_KEY:-}}"
DEFAULT_KUBECONFIG_PATH="${KUBECONFIG:-~/.kube/config}"
SERVER_KUBECONFIG_PATH="${SERVER_KUBECONFIG_PATH:-${DEFAULT_KUBECONFIG_PATH}}"
DEFAULT_KUBE_CONTEXT="$(kubectl config current-context 2>/dev/null || true)"
SERVER_KUBECONFIG_CONTEXT="${SERVER_KUBECONFIG_CONTEXT:-${DEFAULT_KUBE_CONTEXT}}"
SERVER_KUBERNETES_CLUSTER_NAME="${SERVER_KUBERNETES_CLUSTER_NAME:-${SERVER_KUBECONFIG_CONTEXT}}"
SERVER_KUBERNETES_API_SERVER="${SERVER_KUBERNETES_API_SERVER:-}"
SERVER_KUBERNETES_IN_CLUSTER="${SERVER_KUBERNETES_IN_CLUSTER:-true}"

if [[ "${SERVER_KUBERNETES_IN_CLUSTER}" != "true" && "${SERVER_KUBERNETES_IN_CLUSTER}" != "false" ]]; then
  echo "SERVER_KUBERNETES_IN_CLUSTER must be 'true' or 'false' (received '${SERVER_KUBERNETES_IN_CLUSTER}')." >&2
  exit 1
fi

if [[ "${SERVER_KUBERNETES_IN_CLUSTER}" == "true" ]]; then
  SERVER_KUBECONFIG_PATH=""
  SERVER_KUBECONFIG_CONTEXT=""
  SERVER_KUBERNETES_CLUSTER_NAME=""
  SERVER_KUBERNETES_API_SERVER=""
fi

echo "Command:                  ${COMMAND}"
echo "Namespace:                ${NAMESPACE}"
echo "Server image:             ${SERVER_IMAGE}"
echo "Client image:             ${CLIENT_IMAGE}"
echo "Server replicas (start):  ${SERVER_REPLICAS}"
echo "Server service type:      ${SERVER_SERVICE_TYPE}"
echo "Server config ConfigMap:  ${SERVER_CONFIGMAP_NAME}"
echo "Server config mount dir:  ${SERVER_CONFIG_MOUNT_PATH}"
echo "Server METIS_CONFIG path: ${SERVER_METIS_CONFIG_PATH}"
echo

if ! command -v kubectl >/dev/null 2>&1; then
  echo "kubectl is required but was not found in PATH. Install kubectl and configure access to your cluster." >&2
  exit 1
fi

# Make sure kubectl works
kubectl version >/dev/null

generate_server_config() {
  cat <<EOF
[metis]
namespace = "${NAMESPACE}"
worker_image = "${CLIENT_IMAGE}"
server_hostname = "server.${NAMESPACE}.svc.cluster.local"
OPENAI_API_KEY = "${SERVER_OPENAI_API_KEY}"

[service.repositories]
[service.repositories."dourolabs/metis"]
remote_url = "https://github.com/dourolabs/metis.git"
default_branch = "main"
github_token = "${GH_TOKEN}"

[background]
[[background.agent_queues]]
name = "swe"
prompt = """You are a software development agent working on an issue, with the goal of merging a patch to resolve it.
You have access to several tools that enable you to do your job.
- Issue tracker -- use the "metis issues" command
- Pull requests -- use the "metis patches" command

**Your issue id is stored in the METIS_ISSUE_ID environment variable.**

You will be run multiple times on the same issue. Whenever you perform an asynchronous action (such as requesting a pull request),
you will need to end the session and wait for completion. In order to track progress between runs, store running notes about your work
in the issue's progress field so future runs know what to do. If you are re-invoked, you will be provided with the current progress value
to remind you where you left off. Please also update the status of the task as you go. Once you start working on the issue, please mark it as in-progress.
Once the necessary patch(es) are merged, please mark the issue as closed.

metis issues update \$METIS_ISSUE_ID --progress <progress> --status <open|in-progress|closed>

Please perform the following steps to gather context about the issue:
1. Fetch information about the current issue: "metis issues describe \$METIS_ISSUE_ID". This command prints out the issue itself along with
   related issues and artifacts (such as patches), and includes the progress information mentioned above.
2. Determine if the issue has been completed already.

Then, if the issue has been resolved,
3. Update the issue tracker to mark the task as closed: "metis issues update \$METIS_ISSUE_ID --status closed

Otherwise, if the issue has not been resolved:
3. Update the issue tracker to mark the task as in-progress (if not already in-progress): "metis issues update \$METIS_ISSUE_ID --status in-progress
4. Implement a patch to address the issue.
5. Submit the patch as a pull request and assign to jayant by running "metis patches create --github --title <title> --description <description> --assignee jayant"

"""

[background.agent_queues.context]
type = "service_repository"
name = "dourolabs/metis"

[[background.agent_queues]]
name = "pm"
prompt = """You are a product manager specifying the engineering tasks required to implement a larger issue.
You have access to several tools that enable you to do your job.
- Issue tracker -- use the "metis issues" command
- Pull requests -- use the "metis patches" command

**Your issue id is stored in the METIS_ISSUE_ID environment variable.**

You will be run multiple times on the same issue. Whenever you perform an asynchronous action (such as requesting a pull request, or creating an issue to be addressed by someone else),
you will need to end the session and wait for completion. In order to track progress between runs, store running notes about your work
in the issue's progress field so future runs know what to do. If you are re-invoked, you will be provided with the current progress value
to remind you where you left off. Please also update the status of the task as you go. Once you start working on the issue, please mark it as in-progress.
Once the necessary patch(es) are merged, please mark the issue as closed.

metis issues update \$METIS_ISSUE_ID --progress <progress> --status <open|in-progress|closed>

Please perform the following steps to gather context about the issue:
1. Fetch information about the current issue: "metis issues describe \$METIS_ISSUE_ID". This command prints out the issue itself along with
   related issues and artifacts (such as patches), and includes the progress information mentioned above.
2. Determine if the issue has been completed already.

Then, if the issue has been resolved,
3. Update the issue tracker to mark the task as closed: "metis issues update \$METIS_ISSUE_ID --status closed

Otherwise, if the issue has not been resolved:
3. Break down the issue into a set of development tasks. Each development task should represent a single medium-sized PR. Each PR should 
   have only one conceptual change -- break down larger tasks into sequences of changes. For example, migrating something to a new framework
   might involve creating the new framework first, then several subsequent PRs that each migrate one chunk of code from the old framework.
   Every PR should leave the repository in a working and reasonable state after completion.
4. For each new task, add it to the issue tracker using "metis issues create". Please specify dependencies between the tasks using the --deps flag.
   Every new task should be a child-of the current issue, and specify blocked-on relations between the new tasks.
   Make sure to provide enough context in the task description for an agent to implement a PR for the task without consulting other resources.
   Set the assignee of each task to "swe" unless an assignee is otherwise specified in the issue.
"""

[background.agent_queues.context]
type = "service_repository"
name = "dourolabs/metis"

[kubernetes]
in_cluster = ${SERVER_KUBERNETES_IN_CLUSTER}
config_path = "${SERVER_KUBECONFIG_PATH}"
context = "${SERVER_KUBECONFIG_CONTEXT}"
cluster_name = "${SERVER_KUBERNETES_CLUSTER_NAME}"
api_server = "${SERVER_KUBERNETES_API_SERVER}"
EOF
}

apply_manifests() {
  cat <<EOF | kubectl apply -f -
---
apiVersion: v1
kind: Namespace
metadata:
  name: ${NAMESPACE}
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: server-sa
  namespace: ${NAMESPACE}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: server-pod-manager
  namespace: ${NAMESPACE}
rules:
  # Allow managing Pods directly (if you create Pod objects)
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["create", "get", "list", "watch", "delete"]
  # Allow reading pod logs (subresource needed for kubectl logs)
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get", "watch"]
  # Allow managing Jobs (if you choose to create Jobs that run client pods)
  - apiGroups: ["batch"]
    resources: ["jobs"]
    verbs: ["create", "get", "list", "watch", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: server-pod-manager-binding
  namespace: ${NAMESPACE}
subjects:
  - kind: ServiceAccount
    name: server-sa
    namespace: ${NAMESPACE}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: server-pod-manager
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: ${SERVER_CONFIGMAP_NAME}
  namespace: ${NAMESPACE}
data:
  ${SERVER_CONFIG_FILE_NAME}: |
$(generate_server_config | sed 's/^/    /')
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: server
  namespace: ${NAMESPACE}
spec:
  replicas: ${SERVER_REPLICAS}
  selector:
    matchLabels:
      app: server
  template:
    metadata:
      labels:
        app: server
    spec:
      serviceAccountName: server-sa
      containers:
        - name: server
          image: ${SERVER_IMAGE}
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: 8080
          env:
            - name: RUST_LOG
              value: info
            # So the server can know which image to use for client pods
            - name: CLIENT_IMAGE
              value: ${CLIENT_IMAGE}
            - name: METIS_CONFIG
              value: ${SERVER_METIS_CONFIG_PATH}
            # Namespace in which to spawn the clients
            - name: TARGET_NAMESPACE
              valueFrom:
                fieldRef:
                  fieldPath: metadata.namespace
            # How clients can reach the server via Cluster DNS
            - name: SERVER_SERVICE_HOSTNAME
              value: "server.${NAMESPACE}.svc.cluster.local"
          volumeMounts:
            - name: server-config
              mountPath: ${SERVER_CONFIG_MOUNT_PATH}
              readOnly: true
      volumes:
        - name: server-config
          configMap:
            name: ${SERVER_CONFIGMAP_NAME}
---
apiVersion: v1
kind: Service
metadata:
  name: server
  namespace: ${NAMESPACE}
spec:
  selector:
    app: server
  ports:
    - name: http
      port: 80
      targetPort: 8080
  type: ${SERVER_SERVICE_TYPE}
EOF
}

scale_server() {
  local replicas="$1"
  echo "Scaling server deployment to ${replicas} replica(s)..."
  # Avoid failing if the deployment doesn't exist yet
  if kubectl get deployment/server -n "${NAMESPACE}" >/dev/null 2>&1; then
    kubectl scale deployment/server --replicas="${replicas}" -n "${NAMESPACE}"
  else
    echo "Deployment 'server' not found in namespace ${NAMESPACE}, skipping scale."
  fi
}

case "${COMMAND}" in
  start)
    echo "Applying manifests and starting server..."
    apply_manifests
    # Ensure desired replicas (in case Deployment existed with different replicas)
    scale_server "${SERVER_REPLICAS}"
    echo
    echo "Done. Current resources:"
    kubectl get pods,svc -n "${NAMESPACE}"
    ;;

  stop)
    echo "Stopping server (scaling to 0 replicas)..."
    scale_server 0
    echo
    echo "Server scaled down. Current pods:"
    kubectl get pods -n "${NAMESPACE}" || true
    ;;

  status)
    echo "Checking server status..."

    control_plane_ip="$(kubectl get nodes -o wide | awk 'NR>1 && $3 ~ /control-plane/ {print $6; exit}')"
    if [[ -z "${control_plane_ip}" ]]; then
      control_plane_ip="$(kubectl get nodes -o wide | awk 'NR==2 {print $6}')"
    fi

    svc_output="$(kubectl get svc server -n "${NAMESPACE}" 2>/dev/null || true)"
    server_port="$(kubectl get svc server -n "${NAMESPACE}" -o jsonpath='{.spec.ports[0].nodePort}' 2>/dev/null || true)"

    if [[ -z "${server_port}" ]]; then
      server_port="$(kubectl get svc server -n "${NAMESPACE}" -o jsonpath='{.spec.ports[0].port}' 2>/dev/null || true)"
    fi

    if [[ -n "${control_plane_ip}" && -n "${server_port}" ]]; then
      echo "server is running on http://${control_plane_ip}:${server_port}"
    else
      echo "Unable to determine server endpoint."
    fi

    if [[ -n "${svc_output}" ]]; then
      echo
      echo "${svc_output}"
    else
      echo "Service 'server' not found in namespace ${NAMESPACE}."
    fi
    ;;

  destroy)
    echo "Destroying namespace '${NAMESPACE}' (this will delete server, clients, RBAC, services, etc.)..."
    kubectl delete namespace "${NAMESPACE}" --ignore-not-found
    echo "Namespace '${NAMESPACE}' deleted (if it existed)."
    ;;

  restart)
    echo "Restarting server..."
    echo "- Scaling existing pods down to 0..."
    scale_server 0
    echo "- Reapplying manifests to pick up new definitions..."
    apply_manifests
    echo "- Scaling server back to ${SERVER_REPLICAS} replica(s)..."
    scale_server "${SERVER_REPLICAS}"
    echo
    echo "Restart complete. Current resources:"
    kubectl get pods,svc -n "${NAMESPACE}"
    ;;

  *)
    echo "Unknown command: ${COMMAND}"
    echo "Usage: $0 [start|stop|status|restart|destroy]"
    exit 1
    ;;
esac

echo
echo "Done."
