# Hydra

Hydra is an open-source AI orchestration layer. You open a chat, you say what you want, and a fleet of agents goes off and does the work — filing issues, writing documents and code, opening PRs, and reporting back.

![Hydra Dashboard](docs/images/readme-dashboard-2.gif)

## Getting Started

Hydra ships a single-player mode that bundles the CLI, server, and web dashboard into one binary (`hydra-single-player`). This is the easiest way to get started.

### Prerequisites

- **Rust** (stable toolchain) — install via [rustup](https://rustup.rs/).
- **pnpm** - installation instructions [here](https://pnpm.io/installation).
- **Docker** (recommended) — needed for the Docker job engine. See <https://docs.docker.com/get-docker/>.
- **Linux build deps:** `pkg-config` and OpenSSL headers (e.g. `apt install pkg-config libssl-dev`).

### 1. Clone and build

```bash
git clone https://github.com/dourolabs/hydra.git
cd hydra
cargo build -p hydra-single-player
```

The build links the React dashboard into the binary, so no separate frontend setup is needed.

Add the binary to your path:

```bash
mkdir -p ~/.local/bin
cp target/debug/hydra ~/.local/bin/hydra
# or
export PATH="$PATH:$(realpath ./target/debug/)"
```

Verify it's on your path:

```bash
hydra --version
```

### 2. Initialize the server

```bash
hydra server init
```

This walks you through an interactive setup: choosing a username, job engine (Docker or local), AI model (Claude or Codex), API keys, and a GitHub PAT.
When Docker is selected as the job engine, the init command automatically builds a Docker image for agents to run in.
When it finishes, the server is running and the dashboard is configured to talk to it.
You can use the `hydra server` command to start/stop/check the status of the server.

⚠️ **Warning:** Hydra runs agents with `--dangerously-skip-permissions`, so I strongly recommend choosing the Docker engine.
Don't blame me if you choose local and Claude `rm -rf`s your machine.

### 3. Open the dashboard and start a chat

Open <http://localhost:8080/> in your browser and click **New chat**.

Type what you want in plain English — register a repository, fix a bug, add a feature. Chat translates your request into an issue and confirms with the issue ID. The PM agent picks it up, decomposes it if needed, and routes the work to SWE.

Example:

```text
You:  Add the metis-cluster repo at https://github.com/dourolabs/metis-cluster.
Chat: Created `i-abc123` (assigned to pm) to register metis-cluster.

You:  Then add a CI status badge to its README.
Chat: Created `i-def456` (assigned to pm) for the README badge — blocked on `i-abc123`.
```

That's the whole loop. When agents finish work or need your input (a PR to review, a question to answer), they file issues assigned to you and the dashboard surfaces them.

## How chat fits in

Chat is your point of contact. Three other agents do the actual work:

- **`pm`** — receives unassigned issues, investigates, and decomposes them into PR-sized tasks for `swe`.
- **`swe`** — implements code changes and submits patches (pull requests).
- **`reviewer`** — reviews patches and either approves them or requests changes. Can escalate to you.

What chat does:

- Translates your intent into **issue actions** — create, update, drop.
- **Synthesizes status** from issues and patches when you ask ("what changed since yesterday?").
- Can **reconfigure existing agents** — prompts, MCP servers, secrets, retry / concurrency knobs.

## Advanced / power users

The dashboard and the CLI are equivalent surfaces: anything you can do in chat, you can also do directly through `hydra <subcommand>` — and so can the agents. A core design principle of Hydra is that *agents and humans have equal access*.

If you'd rather skip the dashboard and chat from a terminal, `hydra chat` opens a conversation with the chat agent directly. Pass `--prompt "<message>"` for one-shot use, or omit it for an interactive REPL.

### Issues

All work in Hydra is represented by issues. Issues can be assigned to either agents or users. If assigned to agents, the system will automatically spawn sessions to work on the issue. Issues have a progress field that agents automatically update with the current status of the work.

Issues have child-of / blocked-on relationships between them. Hydra uses these to track which issues are ready to be worked on. 

### Agents

Hydra comes with four default agents, created automatically during `hydra server init`:

- **`chat`** -- Conversational interface. Your default point of contact; translates intent into issue actions.
- **`swe`** -- Software engineering agent. Implements code changes, submits patches, and responds to review feedback.
- **`pm`** -- Product manager agent. Breaks down complex features into smaller subtasks and assigns them.
- **`reviewer`** -- Code review agent. Reviews patches and provides feedback.

Agents are configured on the server settings page, and their prompts and memory are stored in the document store.

### Documents

The document store is a shared space for markdown artifacts -- design docs, runbooks, agent prompts / memory, and other reference material.
Check out the documents tab of the frontend to see what's in the store and edit any documents.

### Git Repositories and Branch Management

Repositories are registered with Hydra so agents know where to work. Each issue and task gets tracking branches pushed to the remote:

- `hydra/<issue-id>/base` -- where work on the issue started
- `hydra/<issue-id>/head` -- the current head of work for the issue

This allows sequential agents working on the same issue to pick up where the previous one left off. You can check out any of these branches to inspect the state of work at any point.

## Code Overview

| Crate | Description |
| --- | --- |
| `hydra` | CLI with subcommands for issues, patches, repos, documents, and more. |
| `hydra-server` | Axum HTTP API, background workers, and job engine (Docker or local). Handles persistence, scheduling, and GitHub integration. |
| `hydra-common` | Shared models and types used across all crates. |
| `hydra-bff` | Backend-for-frontend layer: auth routes, API proxy, and embedded frontend serving. |
| `hydra-single-player` | All-in-one binary bundling CLI + server + BFF for local single-player use. |
| `hydra-web` | React 19 frontend with a dark terminal-inspired UI. A pnpm monorepo containing a typed API client (`@hydra/api`), component library (`@hydra/ui`), and the SPA (`@hydra/web`). |
