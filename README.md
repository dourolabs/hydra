# Hydra

Hydra is an open-source AI orchestration layer that lets developers coordinate dozens of agents working simultaneously across tasks, issues, and environments.
You assign work through an issue tracker, and Hydra automatically spins up agents to implement.
You survey their progress, review their work, and offer course corrections as needed.

![Hydra Dashboard](docs/images/readme-dashboard-2.gif)

## Getting Started

Hydra ships a single-player mode that bundles the CLI, server, and web dashboard into one binary (`hydra-single-player`). This is the easiest way to get started.

### 1. Clone and build

```bash
git clone https://github.com/dourolabs/metis.git
cd metis
cargo build -p hydra-single-player
```

Add the binary to your path:

```bash
cp target/debug/hydra ~/.local/bin/hydra
# or
export PATH="$PATH:$(realpath ./target/debug/)"
```

### 2. Initialize the server

```bash
hydra server init
```

This walks you through an interactive setup: choosing a username, job engine (Docker or local), AI model (Claude or Codex), API keys, and a GitHub PAT.
When Docker is selected as the job engine, the init command automatically builds a Docker image for agents to run in.
When it finishes, the server is running and the CLI is configured to talk to it.
You can use the `hydra server` command to start/stop/check the status of the server.

⚠ **Warning:** Hydra runs with agents with `--dangerously-skip-permissions`, so I strongly recommend choosing the Docker engine. Don't blame me if you choose local and Claude `rm -rf`s your machine.

### 3. Add a git repository

Open the frontend at http://localhost:8080/ and click "Create Issue".
Tell the agent "add the git repo (git url)  ".
The agent will register the repo in the system and additionally work on a Dockerfile with the dependencies your repo needs.
You can register as many git repositories as you'd like.

### 4. Start Working

Simply click "Create Issue" and describe what you want done.
The agents will automatically break down your issue into subtasks, identify the right repositories for each one, make changes and submit PRs.
When agents have work for you to review, they'll make issues assigned to you.

## Core Concepts

A core design principle of Hydra is that *agents and humans have equal access*.
All of the functionality described below is available to your agents.

### Issues

All work in Hydra is represented by issues. Issues are the fundamental unit of work, assigned to either agents or users. 
Issues have a status, which is typically: `Open`, `InProgress`, `Closed` or `Failed`.
They form a graph with two types of relationships: `blocked-on` (issue X cannot start until Y is closed) and `child-of` (issue X is a subtask of Y).
The system uses this graph to determine which issues are ready to work on, and automatically spawns agents for ready issues.

When an agent starts working on an issue, it sets the status to `InProgress`. When done, it sets it to `Closed`. If the agent's session ends while the issue is still `InProgress` (e.g., waiting for a code review), another agent can pick it up later with the full git state preserved.

### Agents

Hydra comes with three default agents, created automatically during `hydra server init`:

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
| `hydra-web` | React 19 frontend with a dark terminal-inspired UI. A pnpm monorepo containing a typed API client (`@hydra/api`), component library (`@hydra/ui`), and the SPA + Hono BFF server (`@hydra/web`). |
