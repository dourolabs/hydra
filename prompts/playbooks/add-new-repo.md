# Add a new repo

Requirements: the issue must specify the git remote to connect, eg. https://github.com/foo/bar.git

Required workflow:
0. Determine if this is the first time being run on this issue. Look at child issues to see if the steps in the "initial phase" have
   already been followed. If they have already been followed, skip to the iterative problem solving phase (step 9) below.

Initial Phase:
1. Use "metis repos create" to add the repo to the system, eg "metis repos create foo/bar https://github.com/foo/bar.git"
2. Clone the repository with "metis repos clone".
3. Create an issue to investigate the contents of the repository. The goal is to produce an index document for the repo in the document store
   under /repos/<repo-name>.md. The document must describe what is in the repository, its purpose, the names of major components, service names,
   module names, etc. It should include anything that would help a future agent determine when this repository needs to be used for a task.

   **Preferred method:** Write the index document directly to `$METIS_DOCUMENTS_DIR/repos/<repo-name>.md`. Changes are automatically
   pushed back to the document store when the job completes. Fall back to `metis documents put /repos/<repo-name>.md --file <file>`
   only if filesystem access is unavailable.
4. Create an issue (assigned to **swe**) to produce a docker image for the repository. The goal is to produce a PR adding "Dockerfile.metis" to the repository
   that contains all of the necessary dependencies for building / running / testing the code. The PR should also create a github action that
   builds this image on manual workflow trigger (`workflow_dispatch` only — no cron/schedule) and pushes it to a container registry. Look at the repository to see
   how other docker images (if any) are built and pushed and follow the same pattern here.

   ### Required: GitHub Action workflow configuration

   The GitHub Action workflow must include these settings:

   - **No Docker build caching:** Set `no-cache: true` on the `docker/build-push-action` step.
     Docker layer caching doesn't play well with the `/latest/` release URL used for the metis CLI
     binary — a cached layer will skip re-downloading the binary even when a new version is available.

   - **PR validation trigger:** In addition to `workflow_dispatch`, add a `pull_request` trigger
     filtered to `Dockerfile.metis` path changes. This lets PRs that modify the Dockerfile validate
     that the image still builds successfully. The push step must be conditional on the event type
     so that PR builds only validate (build without pushing):

     ```yaml
     on:
       workflow_dispatch:
         inputs:
           dispatch_description:
             description: "Dispatch description"
             required: true
             type: string
       pull_request:
         paths:
           - "Dockerfile.metis"
     ```

     And in the build step:
     ```yaml
     push: ${{ github.event_name == 'workflow_dispatch' }}
     no-cache: true
     ```

     The registry login step should also be conditional on `workflow_dispatch` since PR builds
     don't need to authenticate:
     ```yaml
     if: github.event_name == 'workflow_dispatch'
     ```

   **The Dockerfile.metis replaces the default metis-worker image for this repo.** It must include everything
   the metis-worker image provides (agent runtime CLIs, supporting tools) PLUS the repo's own build dependencies.
   Use `dourolabs/metis/images/metis-worker.Dockerfile` as the reference for what the base worker image installs.

   ### Required: Agent runtime CLIs

   The metis worker executes jobs by invoking `claude` (Claude Code CLI) or `codex` (OpenAI Codex CLI) inside
   the container. **Without these CLIs, jobs will complete instantly with no log output.** Install them globally
   via npm:

   ```dockerfile
   RUN npm install -g @anthropic-ai/claude-code @openai/codex
   ```

   Verify both are available:
   ```dockerfile
   RUN claude --version && codex --version
   ```

   ### Required: metis CLI binary

   **The Dockerfile must install the `metis` CLI binary so it is available on the agent's PATH.** Download the
   pre-built binary from the metis-releases repository using the `latest` release tag. The metis-releases repo
   maintains a GitHub release tagged `latest` that always points to the current version.

   ```dockerfile
   # Install the metis CLI binary (always pulls the current release)
   RUN curl -fsSL https://github.com/dourolabs/metis-releases/releases/download/latest/metis-x86_64-unknown-linux-gnu \
       -o /usr/local/bin/metis \
       && chmod +x /usr/local/bin/metis
   ```

   **Do NOT hardcode a specific version tag** (e.g., `v0.4.28`) — this causes images to become stale when the
   metis CLI is updated. Using the `latest` tag ensures every image rebuild picks up the current version.

   This ensures the `metis` command is accessible from the CLI inside the container. The binary must be placed in a directory
   that is on the default PATH (e.g. `/usr/local/bin/`). Verify by adding a `RUN metis --version` step in the Dockerfile
   or by documenting that the agent should test `metis --version` after container startup.

   ### Required: Supporting tools for agents

   The agent needs these tools to function effectively inside the container. Install them via apt or appropriate
   package managers:

   - **`ripgrep`** (`rg`) — fast code search, used heavily by agents
   - **`gh`** — GitHub CLI for PR/issue operations (install from https://cli.github.com/)
   - **`op`** — 1Password CLI for secrets management (install via the 1Password apt repository — see below)
   - **`puppeteer`** — headless browser for web fetching (install via `npm install -g puppeteer`; requires
     browser system dependencies — see the metis-worker Dockerfile for the full list of `lib*` packages)
   - **`git`**, **`curl`**, **`jq`**, **`file`**, **`wget`** — standard utilities

   ### Required: 1Password CLI (`op`)

   The 1Password CLI is used by agents for secrets management. Install it from the official 1Password
   apt repository. This must be done as root, before switching to the worker user.

   ```dockerfile
   # Install 1Password CLI
   RUN curl -fsSL https://downloads.1password.com/linux/keys/1password.asc | \
       gpg --dearmor -o /usr/share/keyrings/1password-archive-keyring.gpg && \
       echo "deb [arch=amd64 signed-by=/usr/share/keyrings/1password-archive-keyring.gpg] https://downloads.1password.com/linux/debian/amd64 stable main" \
       > /etc/apt/sources.list.d/1password.list && \
       apt-get update && apt-get install -y --no-install-recommends 1password-cli && \
       rm -rf /var/lib/apt/lists/*
   ```

   Verify it is available:
   ```dockerfile
   RUN op --version
   ```

   ### Required: Non-root worker user and clean WORKDIR

   The container should create a non-root user named `worker` (uid 1000) and set it as the default user.
   This matches the metis-worker convention. The user needs a proper home directory for npm/nvm/cargo config.

   **The Dockerfile must also set WORKDIR to a clean, empty directory** that the worker user can write to.
   The metis agent clones repositories into the container's current working directory (`.`). If the working
   directory is the user's home directory (`/home/worker`), it will contain dotfiles (`.bashrc`, `.nvm/`, etc.)
   and the clone will fail with `"destination '.' is not empty"`.

   Use a dedicated directory like `/src` or `/home/worker/work`:

   ```dockerfile
   RUN useradd -m -s /bin/bash -u 1000 worker
   USER worker
   WORKDIR /src
   ```

   The `WORKDIR` directive creates the directory automatically. If using a path outside the user's home,
   ensure it is writable by the worker user (e.g., create and chown it before switching to `USER worker`).

   ### NOT required: ENTRYPOINT or worker-entrypoint.sh

   **Do NOT add an ENTRYPOINT directive or worker-entrypoint.sh script.** As of PR #1542 (i-bxcvkunct),
   the metis server specifies the container command/args directly in the K8s job spec. Custom worker
   images are now pure dependency containers — they only need to provide the tools, not the startup command.

   If using NVM to install Node.js, ensure tools are available on PATH without sourcing nvm.sh at runtime.
   Create a stable symlink to the nvm-installed node binaries:

   ```dockerfile
   RUN bash -c "source $NVM_DIR/nvm.sh && ln -sf \$(dirname \$(which node)) $NVM_DIR/default"
   ENV PATH="/home/worker/.nvm/default:$PATH"
   ```

   Alternatively, if Node.js is installed via apt/nodesource (not NVM), it will already be on PATH and
   no additional configuration is needed.

   ### GLIBC compatibility

   The pre-built metis binary requires GLIBC 2.38 or newer. Common Debian-based images and
   their GLIBC versions:
   - `debian:bookworm` (Debian 12) — GLIBC 2.36 — **TOO OLD, will fail with `GLIBC_2.38 not found`**
   - `debian:trixie` (Debian 13) — GLIBC 2.41 — **compatible**

   When choosing a base image for Dockerfile.metis, ensure it provides GLIBC >= 2.38. If the project needs an older
   Debian version for other reasons, use a multi-stage build where the final stage uses `debian:trixie` (or newer).
   Use a specific dated tag for reproducibility (e.g., `debian:trixie-20250203`).

5. Create a **separate** issue (assigned to the **issue's creator**, not to swe) to wait for the Docker image build
   to complete. This must be a distinct issue from step 4 because agents do not currently have the GitHub secrets
   required to trigger or monitor `workflow_dispatch` actions. The creator should manually trigger the GitHub Action
   and confirm the image builds successfully before closing this issue.

   This issue should be **blocked on** the step 4 issue (Dockerfile PR) so it only becomes actionable after the PR
   is merged.

6. Create an issue as a follow up to (5) to update the metis repo with the new image name. "metis repos update <repo-name> --default-image <image-name>"
7. Create an issue as a follow up to (6) that runs build / test / lint (as applicable) in the new repo. Ask the agent to report back any
   problems it encounters in the issue itself for further analysis.
8. End the session -- another agent will pick up this issue once the child issues above have completed.

Iterative problem solving phase:
9. Inspect the results from the build / test / lint issue. Look at the issue itself, and if needed, look at logs from the agent run using
   "metis jobs logs <issue-id>".
10. If the agent failed to successfully build / test / lint in the repo, determine if the docker image was the problem. If so, return to
    step (4) above and include instructions in the issue to address the problems.
11. Otherwise, the issue is done.

Troubleshooting:
- **Agent jobs fail to start or produce no output for a repo with a custom image:** The repo's custom
  default-image may be broken (e.g., missing agent CLIs). Clear it with
  `metis repos update <repo-name> --clear-default-image` so jobs fall back to the default metis-worker
  image. Then create a new task to fix the Dockerfile.metis. Once the fixed image is rebuilt, set
  the default-image again with `metis repos update <repo-name> --default-image <image-name>`.
- **Jobs spawn but complete instantly with no logs:** The Docker image is likely missing the agent
  runtime CLIs (`@anthropic-ai/claude-code` and `@openai/codex`). See step 4 above for required installs.