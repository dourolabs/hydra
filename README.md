# Metis

Metis is an experimental AI-orchestrator: a Rust CLI drives an Axum-based
control plane that schedules autonomous jobs onto Kubernetes worker pods.
The CLI (`metis`) is how humans interact with the system (spawning work,
tailing logs, submitting issues and patches), while `metis-server` stores job state,
coordinates background agents, and talks to Kubernetes to launch workers.

> Looking for coding conventions or release expectations? See `AGENTS.md`.

## Repository layout

| Path | Description |
| --- | --- |
| `metis` | End-user CLI with subcommands such as `spawn`, `jobs`, `logs`, `patches`, `issues`, `chat`, and the TUI `dashboard`. |
| `metis-server` | Axum HTTP API plus background agents and Kubernetes `Job` engine. Responsible for job orchestration, persistence, and webhooks. |
| `metis-common` | Shared models (`MetisId`, job/log/issue/patch types, env var constants) used by both crates. |
| `images/` | Dockerfiles for the server and worker images. |
| `scripts/` | Helper scripts (cluster bootstrap, Docker builds, worker entrypoint). |
| `OPERATIONS.md` | Operations guide covering CI failure auto-review behavior, metrics, and alert hooks. |
| `config.toml.sample` files | Copy to `config.toml` (per crate) to override defaults. |

## Prerequisites

- Rust toolchain (1.77+ recommended) and Cargo.
- Kubernetes cluster credentials (`kubectl` configured + the `kube` Rust client can talk to it).
- Docker (for building worker/server images) and, for local clusters, [`kind`](https://kind.sigs.k8s.io/).
- An `OPENAI_API_KEY` (export it or set it inside the server config).

## Building & quick verification

```bash
cargo check --workspace
cargo build --workspace --all-targets
cargo test --workspace
```

## Configuration

### CLI (`metis`)

1. Copy the sample: `cp metis/config.toml.sample metis/config.toml`.
2. Edit `[server].url` to point at your `metis-server` instance, or export
   `METIS_SERVER_URL`. The default points to `http://localhost:8080`.
3. Optional: run `metis --help` to see every subcommand or inspect
   `metis/src/main.rs`.

The CLI reads `--config <file>` if passed, otherwise `~/metis/config.toml`.

#### Natural language chat

`metis chat` opens an interactive Codex session that already knows how to call the
`metis` CLI and starts in the current workspace. Codex will greet you and wait for
your first instruction. For a single-turn question, pass `--prompt`:

```bash
metis chat --prompt "What jobs are running right now?"
```

Both modes launch Codex in the current workspace, inject a description of the
available subcommands, and set `METIS_SERVER_URL` so every CLI call targets the
same server as the parent process. Pass `--model <name>` to override the Codex
model or `--full-auto` to forward Codex's `--full-auto` flag and let it run
commands without manual approvals.

### Server (`metis-server`)

1. `cp metis-server/config.toml.sample metis-server/config.toml`.
2. Fill in:
   - `[metis]` namespace, worker image, and server hostname.
   - `OPENAI_API_KEY` (or export the `OPENAI_API_KEY` env var at runtime).
   - Repository metadata in `[service.repositories.<name>]` so that
     background queues know what to check out.
   - `[[background.agent_queues]]` entries if you want autonomous queues
     that automatically create jobs based on prompts.
   - `[background.scheduler.<worker>]` blocks to adjust polling/backoff
     intervals for background workers (see `config.toml.sample` for defaults).
   - `[kubernetes]` connection info (`in_cluster`, `config_path`, etc.).
3. Launch with `METIS_CONFIG=metis-server/config.toml cargo run -p metis-server`.

## Local Development

### Running metis-server in a kind cluster

For local development with a kind (Kubernetes in Docker) cluster:

#### Prerequisites

- Docker installed and the daemon running
- `kind` and `kubectl` binaries installed and available in PATH
- An `OPENAI_API_KEY` environment variable (export it before running the setup scripts)
- Optional: `GH_TOKEN` if you need GitHub repository access.

#### Setup

1. **Create a kind cluster**:
   ```bash
   kind create cluster --name local-dev
   ```

   To delete the cluster later:
   ```bash
   kind delete cluster --name local-dev
   ```

2. **Build Docker images and load them into the kind cluster**:
   ```bash
   ./scripts/docker-build.sh
   ```

   This builds both `metis-server` and `metis-worker` images and loads them into the `local-dev` cluster. You can override the image names or cluster name via environment variables (`WORKER_IMAGE`, `SERVER_IMAGE`, `KIND_CLUSTER_NAME`).

3. **Start the server in the cluster**:
   ```bash
   ./scripts/service.sh start
   ```

   Starting the server will reference the `OPENAI_API_KEY` and `GH_TOKEN` environment variables and provide them to the server -- you can set these on the command line if you haven't exported them.

   This script creates the `metis` namespace, RBAC resources, ConfigMap, and Deployment for the server.

4. **Get the server URL**:
   ```bash
   ./scripts/service.sh status
   ```

   This prints the server endpoint URL that you can use with the CLI (e.g., `http://127.0.0.1:3XXXX`).

5. **Configure the CLI to use the server**:
   Update `metis/config.toml` or set `METIS_SERVER_URL` to the URL from `./scripts/service.sh status`.

#### Common workflows

- **Redeploy after code changes**: Rebuild images and restart the server:
  ```bash
  ./scripts/docker-build.sh && ./scripts/service.sh restart
  ```

- **Stop the server** (without deleting resources):
  ```bash
  ./scripts/service.sh stop
  ```

- **Destroy all resources**:
  ```bash
  ./scripts/service.sh destroy
  ```
  You probably also want to delete the kind cluster afterward:
  ```bash
  kind delete cluster --name local-dev
  ```
