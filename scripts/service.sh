#!/usr/bin/env bash
set -euo pipefail

COMMAND="${1:-start}"

# -------- Configurable settings --------
# Override any of these by exporting them before running the script, e.g.:
#   NAMESPACE=my-app ./setup-server-client.sh

NAMESPACE="${NAMESPACE:-metis}"
SERVER_IMAGE="${SERVER_IMAGE:-metis-server:latest}"
CLIENT_IMAGE="${CLIENT_IMAGE:-metis-worker:latest}"
S3_IMAGE="${S3_IMAGE:-metis-s3:latest}"
SERVER_REPLICAS="${SERVER_REPLICAS:-1}"
# Assignment agent queue defaults; override ASSIGNMENT_AGENT_QUEUE_NAME to
# customize the deployment-wide routing name without editing this script.
ASSIGNMENT_AGENT_QUEUE_NAME="${ASSIGNMENT_AGENT_QUEUE_NAME:-assignment}"

# Service type for external access:
# - LoadBalancer (default) for managed clusters (GKE/EKS/AKS, etc.)
# - NodePort for bare metal / kind / minikube
SERVER_SERVICE_TYPE="${SERVER_SERVICE_TYPE:-LoadBalancer}"
SERVER_CONFIGMAP_NAME="${SERVER_CONFIGMAP_NAME:-metis-server-config}"
SERVER_CONFIG_MOUNT_PATH="${SERVER_CONFIG_MOUNT_PATH:-/etc/metis}"
SERVER_CONFIG_FILE_NAME="${SERVER_CONFIG_FILE_NAME:-config.toml}"
SERVER_METIS_CONFIG_PATH="${SERVER_METIS_CONFIG_PATH:-${SERVER_CONFIG_MOUNT_PATH}/${SERVER_CONFIG_FILE_NAME}}"

S3_SERVICE_NAME="${S3_SERVICE_NAME:-metis-s3}"
S3_SERVICE_PORT="${S3_SERVICE_PORT:-9090}"
S3_CONFIGMAP_NAME="${S3_CONFIGMAP_NAME:-metis-s3-config}"
S3_CONFIG_MOUNT_PATH="${S3_CONFIG_MOUNT_PATH:-/etc/metis-s3}"
S3_CONFIG_FILE_NAME="${S3_CONFIG_FILE_NAME:-config.toml}"
S3_METIS_CONFIG_PATH="${S3_METIS_CONFIG_PATH:-${S3_CONFIG_MOUNT_PATH}/${S3_CONFIG_FILE_NAME}}"
S3_STORAGE_ROOT="${S3_STORAGE_ROOT:-/var/lib/metis/s3}"

POSTGRES_IMAGE="${POSTGRES_IMAGE:-postgres:16-alpine}"
POSTGRES_SERVICE_NAME="${POSTGRES_SERVICE_NAME:-postgres}"
POSTGRES_DB="${POSTGRES_DB:-metis}"
POSTGRES_USER="${POSTGRES_USER:-metis}"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-metis}"
POSTGRES_PORT="${POSTGRES_PORT:-5432}"
POSTGRES_DATA_PATH="${POSTGRES_DATA_PATH:-/var/lib/postgresql/data}"
POSTGRES_SERVICE_HOSTNAME="${POSTGRES_SERVICE_HOSTNAME:-${POSTGRES_SERVICE_NAME}.${NAMESPACE}.svc.cluster.local}"
SERVER_DATABASE_URL="${SERVER_DATABASE_URL:-postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_SERVICE_HOSTNAME}:${POSTGRES_PORT}/${POSTGRES_DB}}"

# Config generation defaults (can be overridden by env vars)
SERVER_OPENAI_API_KEY="${SERVER_OPENAI_API_KEY:-${OPENAI_API_KEY:-}}"
SERVER_CLAUDE_CODE_OAUTH_TOKEN="${SERVER_CLAUDE_CODE_OAUTH_TOKEN:-${CLAUDE_CODE_OAUTH_TOKEN:-}}"
SERVER_GITHUB_APP_ID="${SERVER_GITHUB_APP_ID:-0}"
SERVER_GITHUB_APP_CLIENT_ID="${SERVER_GITHUB_APP_CLIENT_ID:-}"
SERVER_GITHUB_APP_CLIENT_SECRET="${SERVER_GITHUB_APP_CLIENT_SECRET:-}"
SERVER_GITHUB_APP_PRIVATE_KEY="${SERVER_GITHUB_APP_PRIVATE_KEY:-}"
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
echo "S3 image:                 ${S3_IMAGE}"
echo "Server replicas (start):  ${SERVER_REPLICAS}"
echo "Server service type:      ${SERVER_SERVICE_TYPE}"
echo "S3 service:               ${S3_SERVICE_NAME}.${NAMESPACE}.svc.cluster.local:${S3_SERVICE_PORT}"
echo "Postgres image:           ${POSTGRES_IMAGE}"
echo "Postgres service:         ${POSTGRES_SERVICE_NAME}.${NAMESPACE}.svc.cluster.local:${POSTGRES_PORT}"
echo "Postgres database/user:   ${POSTGRES_DB}/${POSTGRES_USER}"
echo "Server config ConfigMap:  ${SERVER_CONFIGMAP_NAME}"
echo "Server config mount dir:  ${SERVER_CONFIG_MOUNT_PATH}"
echo "Server METIS_CONFIG path: ${SERVER_METIS_CONFIG_PATH}"
echo "S3 config ConfigMap:      ${S3_CONFIGMAP_NAME}"
echo "S3 config mount dir:      ${S3_CONFIG_MOUNT_PATH}"
echo "S3 METIS_CONFIG path:     ${S3_METIS_CONFIG_PATH}"
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
server_hostname = "server.${NAMESPACE}.svc.cluster.local"
OPENAI_API_KEY = "${SERVER_OPENAI_API_KEY}"
CLAUDE_CODE_OAUTH_TOKEN = "${SERVER_CLAUDE_CODE_OAUTH_TOKEN}"

[job]
default_image = "${CLIENT_IMAGE}"
cpu_limit = "500m"
memory_limit = "1Gi"

[database]
url = "${SERVER_DATABASE_URL}"

[service.repositories]
[service.repositories."dourolabs/metis"]
remote_url = "https://github.com/dourolabs/metis.git"
default_branch = "main"

[github_app]
app_id = ${SERVER_GITHUB_APP_ID}
client_id = "${SERVER_GITHUB_APP_CLIENT_ID}"
client_secret = "${SERVER_GITHUB_APP_CLIENT_SECRET}"
private_key = """${SERVER_GITHUB_APP_PRIVATE_KEY}"""

[background]
merge_request_followup_agent = "swe"
assignment_agent = "${ASSIGNMENT_AGENT_QUEUE_NAME}"

# Assignment agents need a dedicated queue so operators can tune prompts without
# touching the worker code. Override ASSIGNMENT_AGENT_QUEUE_NAME or edit this
# block to customize prompt wording, policies, or throttling.
[[background.agent_queues]]
name = "${ASSIGNMENT_AGENT_QUEUE_NAME}"
prompt = """You are an agent responsible for routing Metis issues to the right owner.
Your issue id is stored in the METIS_ISSUE_ID environment variable.

Always run "metis issues describe \$METIS_ISSUE_ID" before making changes so you understand the task, its creator, and any dependencies.
Use "metis issues update \$METIS_ISSUE_ID --assignee <handle> ..." to set the assignee.
When you assign the work, keep the human issue creator as the reviewer by noting that in the progress update and routing any follow-up questions to them.

Assignment rules:
- Complex or multi-PR efforts go to @pm.
- Straightforward single-PR tasks go to @swe.
- Route reviews to the task creator so they stay the primary reviewer for any patches.
- If the right owner is unclear, assign @pm with a progress note describing the ambiguity.

Repo requirements:
- Whenever you assign @pm or @swe, you must also set a repo name via "metis repos list" + "metis issues update \$METIS_ISSUE_ID --repo-name <repo>".
- Never assume a repo when the parent issue omitted job settings; leave repo_name unset unless you can confidently select one.

Document every decision in the issue progress log so future agents understand why you chose that assignee and repo.
"""
[[background.agent_queues]]
name = "swe"
prompt = """You are a software development agent working on an issue, with the goal of merging a patch to resolve it.
You have access to several tools that enable you to do your job.
- Issue tracker -- use the "metis issues" command
- todo list -- use the "metis issues todo" command
- Pull requests -- use the "metis patches" command

**Your issue id is stored in the METIS_ISSUE_ID environment variable.**

You are working on a team with multiple agents, any of which can pick up an issue to work on it. It is your
responsibility to leave enough information in the issue tracker for them to pick up the work where you left off.
Other agents will also be initialized with the state of the git repository as you left it, and any uncommitted changes
will be automatically committed on session termination.
Use the todo list, the progress field and the issue status to communicate this information with your team.
When you start working on the issue, you must set the status to in-progress. 
When you finish working on the issue, you must set the status to closed.

metis issues update \$METIS_ISSUE_ID --progress <progress> --status <open|in-progress|closed>
metis issues todo \$METIS_ISSUE_ID --add "thing that needs to be done"
metis issues todo \$METIS_ISSUE_ID --done 1

IMPORTANT: if your task is to make a change to the codebase, your task should not be closed until you submit a patch and
the patch is merged. Use 'metis patches create --title <title> --description <description>' to submit the patch.

You may also use the issue tracker to create follow-up issues or request work to be performed by another agent in the system.
These issues will be done in the future, and once done another agent will pick up the current issue and continue working.
If you need to wait for these items to be done, simply end the session and another agent will pick it up when possible.
Some actions, such as requesting a pull request, will create tracking issues for async actions automatically -- e.g., they
create an issue requesting a review.

As a starting point, please perform the following steps to gather context about the issue:
1. Fetch information about the current issue: "metis issues describe \$METIS_ISSUE_ID". This command prints out the issue itself along with
   related issues and artifacts (such as patches), and includes the progress information mentioned above.
2. Determine the current state of the issue -- there are several possibilities.

If the issue is new / no patches have been created yet:
3. Update the issue tracker to mark the task as in-progress (if not already in-progress): "metis issues update \$METIS_ISSUE_ID --status in-progress
4. Implement a patch to address the issue.
5. Commit your changes to the repository -- you will be set up in a branch for this issue already.
6. Submit the patch as a pull request and assign to the issue creator (from the "creator" field in "metis issues describe") by running "metis patches create --github --title <title> --description <description> --assignee <creator>"

If one or more patches have been created:
- If the Patch is Merged, then this task may be complete. However, please look at the review feedback and see if there are any follow-up tasks
   that should be created. You can add these to the issue tracker using "metis issues create". 
- If the patch_status is ChangesRequested (typically from a review left without closing the PR), keep working on the same patch: push additional commits
  to the existing branch and do not create a new patch. After addressing all comments, run
  "metis patches update --patch-id <PATCH_ID> --status Open" to reopen the patch for review.
- If the Patch is Closed, then there is significant feedback and the patch needs to be reworked
   and resubmitted. Please make the needed updates to the code and resubmit another patch.

Once you have merged all changes needed for this task and all follow-ups have been finished, then this task is complete.
Update the issue tracker to mark the task as closed: "metis issues update \$METIS_ISSUE_ID --status closed

"""

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

[[background.agent_queues]]
name = "pm2"
prompt = """You are a product manager agent that turns a high-level issue into clear, PR-sized engineering tasks.
You do not implement code. You investigate, research, and plan.
Your output is a set of new issues in the tracker plus concise state in the current issue.

Tools you can use:
- Issue tracker -- use the "metis issues" command
- Todo list -- use the "metis issues todo" command
- Pull requests -- use the "metis patches" command (read-only for status)

**Your issue id is stored in the METIS_ISSUE_ID environment variable.**

Operating principles:
- Keep tasks small: one conceptual change per PR, medium size, shippable.
- Each task must leave the repo in a working state.
- Prefer sequencing over mega-tasks; use dependencies explicitly.
- Capture assumptions and open questions in the progress field.
- Use outside research when needed (APIs, standards, competitors), and cite the source link in progress notes.

Required workflow:
1) Read the issue: "metis issues describe \$METIS_ISSUE_ID".
2) If already resolved (merged patch or explicit resolution), close the issue:
   "metis issues update \$METIS_ISSUE_ID --status closed"
3) Otherwise mark in-progress and store a short working note:
   "metis issues update \$METIS_ISSUE_ID --status in-progress --progress \"...\""

Context gathering:
- Scan repo docs and relevant code paths (AGENTS.md, README, DESIGN.md, module folders).
- Identify unknowns and risks; if clarification is required, create a follow-up issue or a dedicated "clarify" task.
- Do outside research for unfamiliar domains, and summarize key findings briefly.

Task breakdown:
- Produce 2-6 tasks unless the issue is trivial.
- Each task description must include:
  * Goal and user-visible outcome
  * Scope (what is in / out)
  * Key files or directories to touch
  * Acceptance criteria and required tests
  * Dependencies (blocked by or blocks)
- Create tasks as child issues with "metis issues create ... --parent \$METIS_ISSUE_ID".
- Use "--deps" to encode ordering between tasks.
- Assign tasks to "swe" unless the issue specifies a different assignee.

Progress tracking:
- Use the todo list to track your own steps: "metis issues todo \$METIS_ISSUE_ID --add ...".
- After creating tasks, update the progress field with:
  * Short plan summary
  * Task list with issue IDs and dependencies
  * Any open questions or research links

If you trigger any asynchronous work (e.g., waiting on created tasks), end the session so you can be re-run later.
Once all tasks are completed and merged, close the parent issue.
"""

[kubernetes]
in_cluster = ${SERVER_KUBERNETES_IN_CLUSTER}
config_path = "${SERVER_KUBECONFIG_PATH}"
context = "${SERVER_KUBECONFIG_CONTEXT}"
cluster_name = "${SERVER_KUBERNETES_CLUSTER_NAME}"
api_server = "${SERVER_KUBERNETES_API_SERVER}"
EOF
}

generate_s3_config() {
  cat <<EOF
[server]
bind_host = "0.0.0.0"
bind_port = ${S3_SERVICE_PORT}

[storage]
root_dir = "${S3_STORAGE_ROOT}"
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
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "list"]
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
apiVersion: v1
kind: ConfigMap
metadata:
  name: ${S3_CONFIGMAP_NAME}
  namespace: ${NAMESPACE}
data:
  ${S3_CONFIG_FILE_NAME}: |
$(generate_s3_config | sed 's/^/    /')
---
apiVersion: v1
kind: Service
metadata:
  name: ${POSTGRES_SERVICE_NAME}
  namespace: ${NAMESPACE}
spec:
  selector:
    app: ${POSTGRES_SERVICE_NAME}
  ports:
    - name: postgres
      port: ${POSTGRES_PORT}
      targetPort: 5432
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${POSTGRES_SERVICE_NAME}
  namespace: ${NAMESPACE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ${POSTGRES_SERVICE_NAME}
  template:
    metadata:
      labels:
        app: ${POSTGRES_SERVICE_NAME}
    spec:
      containers:
        - name: postgres
          image: ${POSTGRES_IMAGE}
          ports:
            - containerPort: 5432
          env:
            - name: POSTGRES_DB
              value: "${POSTGRES_DB}"
            - name: POSTGRES_USER
              value: "${POSTGRES_USER}"
            - name: POSTGRES_PASSWORD
              value: "${POSTGRES_PASSWORD}"
          resources:
            requests:
              cpu: 400m
              memory: 1Gi
            limits:
              cpu: 400m
              memory: 1Gi
          volumeMounts:
            - name: postgres-data
              mountPath: ${POSTGRES_DATA_PATH}
      volumes:
        - name: postgres-data
          emptyDir: {}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${S3_SERVICE_NAME}
  namespace: ${NAMESPACE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ${S3_SERVICE_NAME}
  template:
    metadata:
      labels:
        app: ${S3_SERVICE_NAME}
    spec:
      containers:
        - name: metis-s3
          image: ${S3_IMAGE}
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: ${S3_SERVICE_PORT}
          env:
            - name: METIS_CONFIG
              value: ${S3_METIS_CONFIG_PATH}
          resources:
            requests:
              cpu: 200m
              memory: 256Mi
            limits:
              cpu: 500m
              memory: 512Mi
          volumeMounts:
            - name: s3-config
              mountPath: ${S3_CONFIG_MOUNT_PATH}
              readOnly: true
            - name: s3-data
              mountPath: ${S3_STORAGE_ROOT}
      volumes:
        - name: s3-config
          configMap:
            name: ${S3_CONFIGMAP_NAME}
        - name: s3-data
          emptyDir: {}
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
          resources:
            requests:
              cpu: 400m
              memory: 1Gi
            limits:
              cpu: 400m
              memory: 1Gi
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
  name: ${S3_SERVICE_NAME}
  namespace: ${NAMESPACE}
spec:
  selector:
    app: ${S3_SERVICE_NAME}
  ports:
    - name: http
      port: ${S3_SERVICE_PORT}
      targetPort: ${S3_SERVICE_PORT}
  type: ClusterIP
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
