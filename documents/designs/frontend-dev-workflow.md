# Design: Frontend Development Workflow for Metis (v2)

## Problem Statement

Frontend development requires a tight visual feedback loop. As an agent builds out components, pages, or layouts, the developer needs to **see** the result and provide spatial, visual feedback — not just code-level comments. Metis's current asynchronous model (issue -> agent job -> patch -> PR review) works well for backend/infra work but creates friction for frontend tasks because:

1. **No live preview**: Workers run in Kubernetes pods with no exposed ports. The developer cannot see the running application until a PR is merged and deployed.
2. **Screenshots are static**: The existing Puppeteer-based screenshot workflow captures a single snapshot, but the developer cannot interact with the page, test responsiveness, or explore different states.
3. **Feedback is code-level**: PR comments operate on diffs, not on visual output. Saying "move the button 10px left" requires the developer to translate visual intent into code-level instructions.
4. **Iteration is slow**: Each round of feedback requires a new agent job, which means minutes of latency per iteration vs. the seconds-level feedback loop developers expect.

## Goals

- Enable developers to **see a live, interactive preview** of frontend changes made by agents, **before** a PR is submitted.
- Allow developers to **provide text feedback** on the rendered output that agents can act on.
- Keep iteration latency under 2 minutes for visual changes.
- Work within Metis's existing architecture (Kubernetes workers, issue/patch model, CLI-first interaction).
- Support both simple (static HTML/CSS) and complex (React/Next.js/Vite) frontend projects, including SSR and dev server HMR.
- Reuse existing Metis conventions for human-in-the-loop review (child issue pattern).

## Non-Goals

- Structured visual annotations (clickable feedback overlays) — future enhancement.
- Real-time collaborative editing (like Figma or Google Docs).
- In-browser code execution (like StackBlitz WebContainers).
- Replacing the existing PR review workflow — previews are a pre-PR step.
- New issue statuses — we reuse the existing child-issue review convention.

## Industry Context

| Approach | Examples | Key Mechanism | Fit for Metis |
|----------|----------|---------------|---------------|
| Deploy-per-commit preview URL | Vercel, Netlify | Every push deploys to unique URL; reviewers click link | Conceptually good, but static-only |
| Port-forwarded cloud dev env | Codespaces, Gitpod | Dev server runs in cloud container; port tunneled to user | **Best fit** — full fidelity, leverages existing K8s infra |
| Snapshot-and-diff visual regression | Chromatic/Storybook | CI captures screenshots; side-by-side diff UI | Good complement for later |
| AI conversational preview loop | v0.dev | AI generates code, preview renders inline, user iterates | Aspirational — requires UI investment |

## Proposed Design

### Architecture Overview

The design introduces two new concepts and reuses one existing convention:

1. **Ephemeral Preview Containers** — long-running Kubernetes pods that serve the agent's work-in-progress frontend via a dev server, accessible via a Tailscale-exposed URL.
2. **Preview CLI Commands** — `metis previews` subcommands for creating, listing, and tearing down preview deployments.
3. **Child-Issue Review Convention (reused)** — the agent creates a child issue assigned to the developer when a preview is ready. The developer marks it closed (approved) or failed (needs revision) with feedback in the progress field. This is the same pattern used for code review requests today.

```
+-----------+    +----------------+    +-------------------+    +-------------+
|  Issue    |--->|  Agent Job     |--->| Preview Container |    |  Preview    |
|  created  |    |  (builds UI)  |    | (dev server live) |--->|  URL live   |
+-----------+    +-------+--------+    +-------------------+    +------+------+
                         |                                             |
                         |    +------------------------------+         |
                         +--->| Review child issue created   |<--------+
                              | (assigned to developer)      |
                              +-------------+----------------+
                                            |
                     +----------------------+----------------------+
                     |                                             |
              Developer marks                               Developer marks
              issue CLOSED                                  issue FAILED
              (= approved)                                  (= needs revision)
                     |                                             |
                     v                                             v
              +------------+                          +-----------------+
              | Agent      |                          | Agent respawns, |
              | creates PR |                          | reads feedback, |
              +------------+                          | iterates        |
                                                      +-----------------+
```

### Component 1: Ephemeral Preview Containers

#### Mechanism

When an agent working on a frontend issue reaches a point where it wants developer feedback, it:

1. Builds or prepares the frontend project.
2. Calls `metis previews create` which:
   a. Starts a new Kubernetes pod running the project's dev server (e.g., `npm run dev`).
   b. Creates a Kubernetes Service for the pod.
   c. Annotates the Service with `tailscale.com/expose: "true"` and a unique hostname.
   d. Returns the preview URL (Tailscale hostname).
3. A review child issue is automatically created (see Component 3).
4. The agent session ends; the preview pod stays alive for developer review.

#### Implementation Details

**Pod Lifecycle**

Unlike agent worker pods (Kubernetes Jobs with `restartPolicy: Never`), preview pods are long-running:

- Created as a **Deployment** (replicas: 1) or a **Pod** directly, not a Job.
- Run the project's dev server command (configurable, e.g., `npm run dev`, `npx vite --host 0.0.0.0`).
- Stay alive until explicitly torn down via `metis previews delete` or automatic cleanup.
- Mount the agent's working directory (code + node_modules) from a shared volume or a snapshot.

**Code Transfer**

The agent's workspace (code changes, built artifacts, node_modules) must be available to the preview container. Options:

- **Option A: Shared volume (recommended for V1)** — The agent writes its workspace to a PVC. The preview pod mounts the same PVC in read-only mode and runs the dev server from there.
- **Option B: S3-based transfer** — The agent tars and uploads the workspace to metis-s3. The preview pod downloads and unpacks it on startup. Simpler isolation but adds startup latency.
- **Option C: Git-based** — The preview pod clones the repo at the agent's working branch. Cleanest but requires the agent to push first and doesn't include uncommitted changes.

**Recommendation**: Start with Option A (shared PVC). The agent creates a PVC, writes its workspace to it, and the preview pod mounts it. This gives the fastest startup and avoids duplicating potentially large `node_modules` directories. We can use the existing `local-path` StorageClass.

**Networking**

Preview pods are exposed via Tailscale, using the same pattern as metis-server and metis-s3:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: preview-{issue-id}
  namespace: metis
  annotations:
    tailscale.com/expose: "true"
    tailscale.com/hostname: "metis-preview-{issue-id}"
spec:
  type: LoadBalancer
  selector:
    app: metis-preview
    metis-issue-id: {issue-id}
  ports:
    - port: 80
      targetPort: 3000  # configurable per framework
```

This gives the developer a URL like `https://metis-preview-i-abc123.monster-vibes.ts.net/` accessible on their Tailscale network.

**Resource Limits**

Preview containers need fewer resources than agent workers since they only run a dev server:

```yaml
resources:
  requests:
    cpu: 500m
    memory: 2Gi
  limits:
    cpu: 1000m
    memory: 4Gi
```

**Cleanup**

Preview pods and their associated resources (Service, PVC) are cleaned up:
- Explicitly via `metis previews delete --issue-id {issue-id}`
- Automatically when the parent issue is closed/dropped
- Via TTL: previews older than 24 hours are garbage collected by a background worker

#### New API Endpoints

```
POST /v1/previews
  - issue_id: IssueId
  - directory: String (path in workspace to serve)
  - command: String (dev server command, e.g., "npm run dev")
  - port: u16 (dev server port, default: 3000)
  - framework: Option<String> (react, next, vue, vite, static)
  - image: Option<String> (container image, defaults to same as agent worker)
  -> Returns: { preview_id, preview_url, review_issue_id }

GET /v1/previews/{preview_id}
  -> Returns: preview metadata, URL, status, review issue ID

GET /v1/previews?issue_id={issue_id}
  -> Returns: list of previews for an issue

DELETE /v1/previews/{preview_id}
  -> Tears down preview pod, Service, PVC
```

Note: The `POST` endpoint also auto-creates the review child issue (see Component 3).

#### CLI Commands

```bash
# Agent creates a preview (typically called by agent, not human)
metis previews create --issue-id i-abc123 \
  --directory ./my-app \
  --command "npm run dev -- --host 0.0.0.0 --port 3000" \
  --port 3000 \
  --framework react

# Developer lists previews
metis previews list --issue-id i-abc123

# Developer opens preview URL (prints or opens in browser)
metis previews open i-abc123

# Tear down preview
metis previews delete --issue-id i-abc123

# List all active previews (for admin/cleanup)
metis previews list --all
```

### Component 2: Feedback via Child-Issue Convention

#### Mechanism

This reuses the existing Metis convention for human-in-the-loop review, which is also used for code review (merge request) issues.

When a preview is created, the system automatically creates a child issue assigned to the developer:

```
Title: Review preview for i-abc123: {original issue title}
Type: Task
Assignee: {issue creator}
Parent: {parent issue id} (child-of dependency)
Progress: ""
Description:
  Preview deployed at: https://metis-preview-i-abc123.monster-vibes.ts.net/

  Please review the preview and provide feedback.

  ## How to respond
  - If the preview looks good, close this issue:
    metis issues update {review-issue-id} --status closed --progress "Approved. Looks good."
  - If changes are needed, mark as failed with your feedback:
    metis issues update {review-issue-id} --status failed --progress "Feedback: ..."

  ## Changes made
  {summary from agent}
```

**Developer workflow:**
1. Developer receives notification of the review issue.
2. Developer visits the preview URL and interacts with it.
3. Developer responds:
   - **Approved**: `metis issues update {review-id} --status closed --progress "Looks good"`
   - **Needs changes**: `metis issues update {review-id} --status failed --progress "Move header 10px up, change button color to blue"`

**Agent workflow after developer responds:**
- The spawner detects the child issue status change and respawns the agent on the parent issue.
- The agent reads the review child issue's status and progress:
  - If `closed`: proceed to create a PR/patch.
  - If `failed`: read feedback, make changes, deploy a new preview, create a new review issue.

#### Auto-Creation Automation

The preview review issue auto-creation mirrors the existing merge-request auto-creation in `app_state.rs:1717-1825`. When the `POST /v1/previews` endpoint is called, the server:

1. Creates the preview pod and service.
2. Creates a review child issue assigned to the parent issue's creator.
3. Returns both the preview URL and the review issue ID.

This is implemented as a server-side side effect (similar to `create_merge_request_issue_for_patch`), not as agent-side logic. The agent simply calls `metis previews create` and the rest happens automatically.

### Component 3: Agent Prompt Instructions

Agents working on frontend issues need specific instructions. These are added to the agent prompt configuration in the server config:

```
When working on frontend issues:

1. After implementing changes, create a preview for developer review:
   metis previews create --issue-id $METIS_ISSUE_ID \
     --directory ./path-to-project \
     --command "npm run dev -- --host 0.0.0.0 --port 3000" \
     --port 3000

2. The system will automatically create a review issue for the developer.
   Update the parent issue progress with a summary of changes made:
   metis issues update $METIS_ISSUE_ID --progress "Preview deployed. Changes: {summary}"

3. End your session. The developer will review the preview and respond.

4. When re-spawned after developer feedback:
   - Check child issues for the latest review issue
   - Read its status and progress field:
     - If status is "closed": preview approved. Create a patch/PR with the final code.
     - If status is "failed": read feedback from progress, make changes, create a new preview.

5. When creating the final patch, include a screenshot as an asset:
   node -e "const p=require('puppeteer'); ..."
   metis patches assets create --patch-id <id> --file screenshot.png
```

### Component 4: Server-Side Changes

#### Preview Resource Management

A new `PreviewManager` component in metis-server handles:

1. **Pod creation**: Creates a Kubernetes Deployment (or Pod) with the project's dev server.
2. **Service creation**: Creates a LoadBalancer Service with Tailscale annotations.
3. **PVC management**: Creates/manages the shared volume for code transfer.
4. **Review issue creation**: Auto-creates the child review issue.
5. **Cleanup**: Garbage collects expired previews (background worker).

#### RBAC Extension

The server's Kubernetes role needs additional permissions:

```yaml
rules:
  # Existing permissions...
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["create", "get", "list", "watch", "delete"]
  - apiGroups: [""]
    resources: ["services"]
    verbs: ["create", "get", "list", "watch", "delete"]
  - apiGroups: [""]
    resources: ["persistentvolumeclaims"]
    verbs: ["create", "get", "list", "watch", "delete"]
```

#### Database/Store

Preview metadata stored in the existing store:

```rust
pub struct Preview {
    pub preview_id: PreviewId,
    pub issue_id: IssueId,
    pub review_issue_id: IssueId,
    pub url: String,
    pub command: String,
    pub port: u16,
    pub framework: Option<String>,
    pub status: PreviewStatus,  // Running, Stopped, Failed
    pub created_at: DateTime<Utc>,
}
```

New `PreviewId` variant added to the `MetisId` system (prefix: `pv-`).

#### Spawner Changes

No new issue statuses are needed. The existing spawner behavior handles this correctly:

- When the agent creates a preview and ends its session, the parent issue remains `InProgress`.
- The child review issue is `Open`, assigned to the developer (not an agent), so no agent spawns for it.
- When the developer closes/fails the review issue, the spawner's child-version-change detection resets the retry counter on the parent issue, triggering a respawn.

The only spawner change needed: ensure the spawner does not respawn on the parent issue while a preview is actively running and no review issue has been responded to. This can be done by checking if there are any `Open` review-type child issues — if so, skip spawning. (This is analogous to how `parent_has_running_task()` works.)

## Implementation Plan

### Phase 1: Ephemeral Preview System (MVP)

**Changes required, organized as PR-sized tasks:**

1. **metis-common: Add Preview types and API** — Add `PreviewId` to the ID system, `Preview` model, `PreviewStatus` enum, and request/response types for the preview CRUD endpoints.

2. **metis-server: Preview store and CRUD endpoints** — Add preview storage to the store layer (memory + postgres), implement `POST/GET/DELETE /v1/previews` route handlers.

3. **metis-server: Kubernetes preview manager** — Implement the `PreviewManager` that creates/deletes Kubernetes Deployments, Services (with Tailscale annotations), and PVCs for preview containers. Wire it into the preview CRUD endpoints.

4. **metis-server: Auto-create review issue on preview creation** — When `POST /v1/previews` is called, automatically create a child issue assigned to the parent issue's creator, similar to merge-request auto-creation. Return the review issue ID in the response.

5. **metis-server: Preview cleanup background worker** — Add a background worker that garbage collects previews older than 24 hours and cleans up previews when parent issues are closed.

6. **metis-server: Spawner guard for active previews** — Add logic to skip spawning on an issue if it has an active preview with an unresolved (Open) review child issue.

7. **metis CLI: Add `metis previews` subcommand** — Implement `create`, `list`, `open`, and `delete` subcommands.

8. **metis-cluster: RBAC and resource quota updates** — Update the server's Kubernetes Role to allow creating Deployments, Services, and PVCs. Adjust resource quotas if needed.

9. **Agent prompt updates** — Add frontend-specific workflow instructions to the SWE agent prompt.

### Phase 2: Enhanced Feedback (future)

1. Inject a lightweight JS feedback toolbar into preview pages for point-and-click annotations.
2. Store structured annotations via API, deliver them to agents.

### Phase 3: Visual Diff & Review UI (future)

1. Side-by-side preview comparison in metis-ui.
2. Pixel diffing between preview versions.

## Open Questions

1. **Code transfer mechanism**: The recommended approach (shared PVC) means the agent must write its workspace to the PVC before creating the preview. Should the agent's normal workspace already be on a PVC, or should we add an explicit "snapshot workspace to PVC" step? The current worker pods do not use PVCs.

2. **Dev server command discovery**: How does the agent determine the right dev server command? Options: (a) always specify in the issue, (b) detect from package.json scripts, (c) use framework-specific defaults.

3. **Preview port configuration**: Most frameworks default to port 3000 or 5173. Should we standardize on a single port or allow per-preview configuration? The current design allows per-preview port configuration.

4. **Tailscale hostname limits**: Tailscale hostnames have character restrictions. Need to verify that issue IDs (e.g., `i-abc123`) are valid in Tailscale hostnames.

5. **Preview pod image**: Should preview pods use the same image as agent workers (which includes all dev tools), or a lighter image? Using the same image is simpler but more resource-heavy.

6. **Concurrent previews**: Should we limit the number of concurrent previews per issue or globally? The resource quota already limits total pods to 100.

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Preview pods consume resources while idle | Cluster resource pressure | TTL-based auto-cleanup (24h); explicit `metis previews delete` |
| Tailscale hostname propagation latency | Developer waits for URL to become live | Test typical propagation time; include status polling in CLI |
| Large node_modules on PVC | Storage pressure on local-path provisioner | Set PVC size limits; consider shared node_modules cache |
| Dev server crashes in preview pod | Preview URL serves errors | Health checks on preview pods; restart policy; agent can redeploy |
| Agent doesn't know right dev server command | Preview fails to start | Detect from package.json; require explicit command in issue |

## Appendix: Existing Capabilities Leveraged

- **Kubernetes Job Engine** (`kubernetes_job_engine.rs`): Existing pattern for creating pods, injecting env vars, managing labels. Extended for Deployments + Services.
- **Tailscale Operator** (metis-cluster): Already deployed, already exposes metis-server and metis-s3 via `tailscale.com/expose` annotation. Same pattern used for preview services.
- **Child-Issue Review Convention**: Already used for merge request (code review) issues. The spawner's child-version-change detection already handles respawning when child issues are updated.
- **Merge-Request Auto-Creation** (`app_state.rs:1717-1825`): Pattern for server-side auto-creation of review issues. Replicated for preview review issues.
- **Puppeteer in Worker Image**: Already installed. Can capture screenshots of previews for PR assets.
- **MetisId System** (`ids.rs`): Type-safe ID generation. Extended with `PreviewId` (prefix: `pv-`).
- **Local-Path StorageClass** (metis-cluster): Dynamic PVC provisioning on local NVMe. Used for preview workspace volumes.
- **Resource Quotas** (metis-cluster): Already enforce pod limits in the metis namespace.
