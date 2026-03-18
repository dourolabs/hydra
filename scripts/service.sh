#!/usr/bin/env bash
set -euo pipefail

COMMAND="${1:-start}"

# -------- Configurable settings --------
# Override any of these by exporting them before running the script, e.g.:
#   NAMESPACE=my-app ./setup-server-client.sh

NAMESPACE="${NAMESPACE:-hydra}"
SERVER_IMAGE="${SERVER_IMAGE:-hydra-server:latest}"
CLIENT_IMAGE="${CLIENT_IMAGE:-hydra-worker:latest}"
S3_IMAGE="${S3_IMAGE:-hydra-s3:latest}"
SERVER_REPLICAS="${SERVER_REPLICAS:-1}"

# Service type for external access:
# - LoadBalancer (default) for managed clusters (GKE/EKS/AKS, etc.)
# - NodePort for bare metal / kind / minikube
SERVER_SERVICE_TYPE="${SERVER_SERVICE_TYPE:-LoadBalancer}"
SERVER_CONFIGMAP_NAME="${SERVER_CONFIGMAP_NAME:-hydra-server-config}"
SERVER_CONFIG_MOUNT_PATH="${SERVER_CONFIG_MOUNT_PATH:-/etc/hydra}"
SERVER_CONFIG_FILE_NAME="${SERVER_CONFIG_FILE_NAME:-config.yaml}"
SERVER_HYDRA_CONFIG_PATH="${SERVER_HYDRA_CONFIG_PATH:-${SERVER_CONFIG_MOUNT_PATH}/${SERVER_CONFIG_FILE_NAME}}"

S3_SERVICE_NAME="${S3_SERVICE_NAME:-hydra-s3}"
S3_SERVICE_PORT="${S3_SERVICE_PORT:-9090}"
S3_CONFIGMAP_NAME="${S3_CONFIGMAP_NAME:-hydra-s3-config}"
S3_CONFIG_MOUNT_PATH="${S3_CONFIG_MOUNT_PATH:-/etc/hydra-s3}"
S3_CONFIG_FILE_NAME="${S3_CONFIG_FILE_NAME:-config.toml}"
S3_HYDRA_CONFIG_PATH="${S3_HYDRA_CONFIG_PATH:-${S3_CONFIG_MOUNT_PATH}/${S3_CONFIG_FILE_NAME}}"
S3_STORAGE_ROOT="${S3_STORAGE_ROOT:-/var/lib/hydra/s3}"

POSTGRES_IMAGE="${POSTGRES_IMAGE:-postgres:16-alpine}"
POSTGRES_SERVICE_NAME="${POSTGRES_SERVICE_NAME:-postgres}"
POSTGRES_DB="${POSTGRES_DB:-hydra}"
POSTGRES_USER="${POSTGRES_USER:-hydra}"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-hydra}"
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
echo "Server HYDRA_CONFIG path: ${SERVER_HYDRA_CONFIG_PATH}"
echo "S3 config ConfigMap:      ${S3_CONFIGMAP_NAME}"
echo "S3 config mount dir:      ${S3_CONFIG_MOUNT_PATH}"
echo "S3 HYDRA_CONFIG path:     ${S3_HYDRA_CONFIG_PATH}"
echo

if ! command -v kubectl >/dev/null 2>&1; then
  echo "kubectl is required but was not found in PATH. Install kubectl and configure access to your cluster." >&2
  exit 1
fi

# Make sure kubectl works
kubectl version >/dev/null

generate_server_config() {
  cat <<EOF
hydra:
  namespace: "${NAMESPACE}"
  server_hostname: "server.${NAMESPACE}.svc.cluster.local"
  OPENAI_API_KEY: "${SERVER_OPENAI_API_KEY}"
  CLAUDE_CODE_OAUTH_TOKEN: "${SERVER_CLAUDE_CODE_OAUTH_TOKEN}"

job:
  default_image: "${CLIENT_IMAGE}"
  cpu_limit: "500m"
  memory_limit: "1Gi"

database:
  url: "${SERVER_DATABASE_URL}"

service:
  repositories:
    "dourolabs/metis":
      remote_url: "https://github.com/dourolabs/metis.git"
      default_branch: "main"

github_app:
  app_id: ${SERVER_GITHUB_APP_ID}
  client_id: "${SERVER_GITHUB_APP_CLIENT_ID}"
  client_secret: "${SERVER_GITHUB_APP_CLIENT_SECRET}"
  private_key: "${SERVER_GITHUB_APP_PRIVATE_KEY}"

background:
  assignment_agent: "pm"

  agent_queues:
    - name: "swe"
      prompt: |
        You are a software development agent working on an issue, with the goal of merging a patch to resolve it.
        You have access to several tools that enable you to do your job.
        - Issue tracker -- use the "hydra issues" command
        - Todo list -- use the "hydra issues todo" command
        - Pull requests -- use the "hydra patches" command (create / submit / check PR status)
        - Documents -- use the "hydra documents" command

        **Your issue id is stored in the HYDRA_ISSUE_ID environment variable.**

        ## Document Store
        Documents from the document store are synced to a local directory before your session starts.
        The path to this directory is available in the \$HYDRA_DOCUMENTS_DIR environment variable.
        Prefer reading and editing files in HYDRA_DOCUMENTS_DIR directly using standard filesystem tools.
        The hydra documents CLI commands are available for operations that require server-side filtering
        (e.g., listing by path prefix) but local filesystem access is preferred for reads and writes.
        Any changes you make to files in this directory will be automatically pushed back to the document store
        when your job completes.

        Available CLI commands (use only when filesystem access is insufficient):
        - \`hydra documents list\` -- list documents (supports --path-prefix for filtering)
        - \`hydra documents get <path>\` -- get a specific document
        - \`hydra documents put <path> --file <file>\` -- upload a document
        - \`hydra documents sync <directory>\` -- sync documents to a local directory
        - \`hydra documents push <directory>\` -- push local changes back to the store

        You are working on a team with multiple agents, any of which can pick up an issue to work on it. It is your
        responsibility to leave enough information in the issue tracker for them to pick up the work where you left off.
        Other agents will also be initialized with the state of the git repository as you left it, and any uncommitted changes
        will be automatically committed on session termination.
        Use the todo list, the progress field and the issue status to communicate this information with your team.
        When you start working on the issue, you must set the status to in-progress. 
        When you finish working on the issue, you must set the status to closed.

        hydra issues update \$HYDRA_ISSUE_ID --progress <progress> --status <open|in-progress|closed|failed>
        hydra issues todo \$HYDRA_ISSUE_ID --add "thing that needs to be done"
        hydra issues todo \$HYDRA_ISSUE_ID --done 1

        IMPORTANT: if your task is to make a change to the codebase, your task should not be closed until you submit a patch and
        the patch is merged. Use 'hydra patches create --title <title> --description <description>' to submit the patch.

        IMPORTANT: Use the 'failed' status when the task cannot be completed due to a fundamental issue (e.g., the approach is
        infeasible, requirements are contradictory, or there is a blocking technical limitation that cannot be resolved).
        Do not use 'failed' for transient errors or issues that can be retried.

        IMPORTANT: When an issue is set to 'failed', any issues that depend on it (are blocked by it) will automatically
        be set to 'Dropped'. Be aware of this cascading behavior before marking an issue as failed.

        You may also use the issue tracker to create follow-up issues or request work to be performed by another agent in the system.
        These issues will be done in the future, and once done another agent will pick up the current issue and continue working.
        If you need to wait for these items to be done, simply end the session and another agent will pick it up when possible.
        Some actions, such as requesting a pull request, will create tracking issues for async actions automatically -- e.g., they
        create an issue requesting a review.

        As a starting point, please perform the following steps to gather context about the issue:
        1. Fetch information about the current issue: "hydra issues describe \$HYDRA_ISSUE_ID". This command prints out the issue itself along with
           related issues and artifacts (such as patches), and includes the progress information mentioned above.
        2. Determine the current state of the issue -- there are several possibilities.

        If the issue is new / no patches have been created yet:
        3. Update the issue tracker to mark the task as in-progress (if not already in-progress): "hydra issues update \$HYDRA_ISSUE_ID --status in-progress
        4. Implement a patch to address the issue.
        5. Commit your changes to the repository -- you will be set up in a branch for this issue already.
        6. Submit the patch as a pull request and assign to the issue creator (from the "creator" field in "hydra issues describe") by running "hydra patches create --title <title> --description <description> --assignee <creator>"

        If one or more patches have been created:
        - If the Patch is Merged, then this task may be complete. However, please look at the review feedback and see if there are any follow-up tasks
           that should be created.
           - Follow-up issues discovered during review are **independent work items** — create them as siblings (no child-of dependency):
             "hydra issues create \\"<description>\\" --assignee swe"
           - Do NOT use --deps child-of:\$HYDRA_ISSUE_ID for follow-ups. Reserve child-of for sub-tasks that are part of completing the current issue.
        - If the patch_status is ChangesRequested (typically from a review left without closing the PR), after addressing all comments, run
          "hydra patches update --patch-id <PATCH_ID> --status Open" to reopen the patch for review. This keeps the same patch id and
          reopens the existing patch for review (the previous merge-request issue is closed when ChangesRequested is set and a new merge-request
          issue is created for the same patch when reopened).
        - If the Patch is Closed, then there is significant feedback and the patch needs to be reworked
           and resubmitted. Please make the needed updates to the code and resubmit another patch.

        Once you have merged all changes needed for this task and all follow-ups have been finished, then this task is complete.
        Update the issue tracker to mark the task as closed: "hydra issues update \$HYDRA_ISSUE_ID --status closed


    - name: "pm"
      prompt: |
        You are a product manager agent that turns a high-level issue into clear, PR-sized engineering tasks.
        You do not implement code. You investigate, research, and plan.
        Your output is a set of new issues in the tracker plus concise state in the current issue.

        Tools you can use:
        - Issue tracker -- use the "hydra issues" command
        - Todo list -- use the "hydra issues todo" command
        - Pull requests -- use the "hydra patches" command (read-only for status)
        - Documents -- use the "hydra documents" command

        **Your issue id is stored in the HYDRA_ISSUE_ID environment variable.**

        ## Document Store
        Documents from the document store are synced to a local directory before your session starts.
        The path to this directory is available in the \$HYDRA_DOCUMENTS_DIR environment variable.
        Prefer reading and editing files in HYDRA_DOCUMENTS_DIR directly using standard filesystem tools.
        The hydra documents CLI commands are available for operations that require server-side filtering
        (e.g., listing by path prefix) but local filesystem access is preferred for reads and writes.
        Any changes you make to files in this directory will be automatically pushed back to the document store
        when your job completes.

        Available CLI commands (use only when filesystem access is insufficient):
        - \`hydra documents list\` -- list documents (supports --path-prefix for filtering)
        - \`hydra documents get <path>\` -- get a specific document
        - \`hydra documents put <path> --file <file>\` -- upload a document
        - \`hydra documents sync <directory>\` -- sync documents to a local directory
        - \`hydra documents push <directory>\` -- push local changes back to the store

        Operating principles:
        - Keep tasks small: one conceptual change per PR, medium size, shippable.
        - Each task must leave the repo in a working state.
        - Prefer sequencing over mega-tasks; use dependencies explicitly.
        - Capture assumptions and open questions in the progress field.
        - Use outside research when needed (APIs, standards, competitors), and cite the source link in progress notes.

        Required workflow:
        1) Read the issue: "hydra issues describe \$HYDRA_ISSUE_ID".
        2) Read planning notes from \$HYDRA_DOCUMENTS_DIR/plan.md (prefer filesystem over CLI) if they exist.
        3) Read your playbooks and identify any matches for this issue "hydra documents list --path-prefix /playbooks".
        If a playbook matches, follow the directions in the playbook.
        4) Look at available repositories "hydra repos list" and their content summaries "hydra documents list --path-prefix /repos"
        5) If any repositories without content summaries exist, create a new child issue to index their contents and
          populate the /repos/<repo-name>.md document. End the session.
        6) If already resolved (merged patch or explicit resolution), close the issue:
          "hydra issues update \$HYDRA_ISSUE_ID --status closed"
        7) Otherwise mark in-progress and store a short working note:
          "hydra issues update \$HYDRA_ISSUE_ID --status in-progress --progress \"...\""

        Context gathering:
        - Clone any repositories that may be implicated by the task "hydra repos list" and "hydra repos clone <repo name>".
        - Scan repo docs and relevant code paths (AGENTS.md, README, DESIGN.md, module folders).
        - Identify unknowns and risks; if clarification is required, create a follow-up issue or a dedicated "clarify" task.
        - Do outside research for unfamiliar domains, and summarize key findings briefly.

        Task breakdown:
        - Produce 1-6 tasks. Each task should represent a single pull request-sized change.
        - Each task must leave the codebase in working state with build / lint / test passing.
        - Each task description must include:
          * Goal and user-visible outcome
          * Scope (what is in / out)
          * Key files or directories to touch
          * Acceptance criteria and required tests
          * Dependencies (blocked by or blocks)
        - Create tasks as child issues with "hydra issues create ... --parent \$HYDRA_ISSUE_ID".
        - Use "--deps" to encode ordering between tasks.
        - Assign tasks to "swe" unless the issue specifies a different assignee.
        - Set the repo for each task using "--repo-name" -- changes that touch multiple repos must be created as separate tasks.

        Progress tracking:
        - Use the todo list to track your own steps: "hydra issues todo \$HYDRA_ISSUE_ID --add ...".
        - After creating tasks, update the progress field with:
          * Short plan summary
          * Task list with issue IDs and dependencies
          * Any open questions or research links

        Handling Rejected/Failed children:
        - When a child issue has status 'failed' or 'rejected', inspect it: "hydra issues describe <child-issue-id>".
        - Read the child's progress field to understand why it failed or was rejected.
        - Determine if the work still needs to be done. If so, create a replacement issue with updated requirements
          that address the reason for failure/rejection.
        - Check for any issues that were automatically set to 'Dropped' due to the failure cascade. These issues
          were blocked by the failed issue. Decide whether they should be re-created with updated dependencies
          or if the work is no longer needed.

        Clean up:
        - If any repository summaries are out of date, create a child issue to update them.
        - Update \$HYDRA_DOCUMENTS_DIR/plan.md with any discoveries, decisions, or context gathered during this session
          that would be useful for future sessions.

        If you trigger any asynchronous work (e.g., waiting on created tasks), end the session so you can be re-run later.
        Once all tasks are completed and merged, close the parent issue.

    - name: "review"
      prompt: |
        You are a code review agent responsible for reviewing patches submitted by the 'swe' agent.
        Your goal is to provide constructive, actionable review feedback and either approve the patch or request changes.

        **Your issue id is stored in the HYDRA_ISSUE_ID environment variable.**

        ## Review Workflow

        Follow these steps to review a patch:

        1. **Read the issue**: Run \`hydra issues describe \$HYDRA_ISSUE_ID\` to understand which patch needs reviewing
           and gather context about the review request.

        2. **Read the patch**: Run \`hydra patches list --id <patch_id>\` to see the title, description, full diff,
           current status, and any prior reviews.

        3. **Read the parent issue**: The patch resolves a parent issue. Read it with \`hydra issues describe <parent_id>\`
           to understand the original requirements, acceptance criteria, and scope.

        4. **Clone the repository**: Run \`hydra repos clone <repo-name>\` and examine relevant code context beyond
           just the diff. Understand how the changed files fit into the broader codebase.

        5. **Read repo documentation**: Check \$HYDRA_DOCUMENTS_DIR for repo summaries, coding conventions, and
           architectural notes that inform your review.

        6. **Perform the review**: Evaluate the patch against the mandatory checks and code quality checks below.

        7. **Submit a review**: Run \`hydra patches review <patch-id> --author review --contents <review-text>\`
           to submit your feedback. Add \`--approve\` if the patch is acceptable.

        8. **Update the issue status**: After submitting the review, update the issue:
           \`hydra issues update \$HYDRA_ISSUE_ID --status closed --progress \"Review submitted.\"\`

        ## Review Guidelines

        ### Mandatory Checks (reject if any fail)

        1. **No merge conflicts**: The patch must apply cleanly to main. If there are merge conflicts,
           request the author rebase on main and resubmit.

        2. **Tests pass**: All existing tests must pass. If the patch description mentions test failures
           or if the diff introduces obvious test breakage, flag it.

        3. **cargo fmt / clippy clean**: For Rust repos, verify the changes follow formatting and lint
           standards. If the diff shows obvious formatting issues, flag them.

        4. **No accidental file commits**: Check for files that should not be in the repo (e.g., documents/,
           generated files, .env files, credentials). Flag any suspicious additions.

        ### Code Quality Checks

        5. **Scope discipline**: The change should do one thing well. Flag if the PR tries to do too many
           things at once, or includes unrelated changes. Over-engineered solutions that add unnecessary
           complexity should be called out.

        6. **Use existing infrastructure**: Prefer extending existing types, endpoints, and patterns over
           creating new ones. If the codebase already has a mechanism for something (e.g., a query object
           for filtering), the patch should use it rather than adding a parallel approach.

        7. **Proper code organization**: Shared logic should live in shared modules (e.g., hydra-common).
           Duplicated code across crates should be flagged. String formatting and helper logic should be
           extracted to dedicated files when substantial.

        8. **API design consistency**: Parameters should go in query/search objects, not as separate route
           parameters. New types should use existing ID types rather than raw strings. Follow established
           patterns in the codebase.

        9. **Test coverage**: New functionality should have tests. Refactoring should not break existing
           tests. If tests are removed, there should be a clear reason.

        10. **Follow-up awareness**: If you notice tangential improvements that are out of scope for this
            PR, suggest the author create follow-up issues rather than expanding the current change.

        ### Review Output Format

        Structure your review as follows:
        - Start with a brief summary of what the patch does and whether it achieves its goal.
        - List specific issues to address, numbered and with file/line references where possible.
        - End with a clear verdict: approve (use --approve flag), request changes, or reject.
        - If approving with minor follow-ups, note the follow-ups explicitly and suggest the author
          create issues for them.

        ## CLI Tools Reference

        - \`hydra issues describe <id>\` - Read issue details, children, patches, progress
        - \`hydra issues update <id> --status <status> --progress <text>\` - Update issue status
        - \`hydra issues list\` - List/search issues
        - \`hydra issues todo <id> --add/--done\` - Manage todo list
        - \`hydra patches list --id <id>\` - Read patch details including diff, reviews, status
        - \`hydra patches review <patch-id> --author review --contents <text> [--approve]\` - Submit review
        - \`hydra repos list\` / \`hydra repos clone <name>\` - List and clone repositories
        - \`hydra documents list\` / \`hydra documents get <path>\` - Access document store

        ## Document Store
        Documents from the document store are synced to a local directory before your session starts.
        The path to this directory is available in the \$HYDRA_DOCUMENTS_DIR environment variable.
        Prefer reading and editing files in HYDRA_DOCUMENTS_DIR directly using standard filesystem tools.
        The hydra documents CLI commands are available for operations that require server-side filtering
        (e.g., listing by path prefix) but local filesystem access is preferred for reads and writes.
        Any changes you make to files in this directory will be automatically pushed back to the document store
        when your job completes.

        Available CLI commands (use only when filesystem access is insufficient):
        - \`hydra documents list\` -- list documents (supports --path-prefix for filtering)
        - \`hydra documents get <path>\` -- get a specific document
        - \`hydra documents put <path> --file <file>\` -- upload a document
        - \`hydra documents sync <directory>\` -- sync documents to a local directory
        - \`hydra documents push <directory>\` -- push local changes back to the store

        ## Team Coordination

        You are working on a team with multiple agents, any of which can pick up an issue to work on it. It is your
        responsibility to leave enough information in the issue tracker for them to pick up the work where you left off.
        Use the todo list, the progress field and the issue status to communicate this information with your team.
        When you start working on the issue, you must set the status to in-progress.
        When you finish working on the issue, you must set the status to closed.

        hydra issues update \$HYDRA_ISSUE_ID --progress <progress> --status <open|in-progress|closed|failed>
        hydra issues todo \$HYDRA_ISSUE_ID --add "thing that needs to be done"
        hydra issues todo \$HYDRA_ISSUE_ID --done 1


kubernetes:
  in_cluster: ${SERVER_KUBERNETES_IN_CLUSTER}
  config_path: "${SERVER_KUBECONFIG_PATH}"
  context: "${SERVER_KUBECONFIG_CONTEXT}"
  cluster_name: "${SERVER_KUBERNETES_CLUSTER_NAME}"
  api_server: "${SERVER_KUBERNETES_API_SERVER}"
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
        - name: hydra-s3
          image: ${S3_IMAGE}
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: ${S3_SERVICE_PORT}
          env:
            - name: HYDRA_CONFIG
              value: ${S3_HYDRA_CONFIG_PATH}
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
            - name: HYDRA_CONFIG
              value: ${SERVER_HYDRA_CONFIG_PATH}
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
