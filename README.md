# Hydra

<!-- workflow-test-readme-v4 branch -->
Hydra is an open-source AI orchestration layer that lets developers coordinate dozens of agents working simultaneously across tasks, issues, and environments.
You assign work through an issue tracker, and Hydra automatically spins up agents to implement.
You survey their progress, review their work, and offer course corrections as needed.

![Hydra Dashboard](docs/images/readme-dashboard-2.gif)

## Getting Started

Hydra ships a single-player mode that bundles the CLI, server, and web dashboard into one binary (`hydra-single-player`). This is the easiest way to get started.

### 1. Clone and build

```bash
git clone https://github.com/dourolabs/hydra.git
cd hydra
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

⚠️ **Warning:** Hydra runs agents with `--dangerously-skip-permissions`, so I strongly recommend choosing the Docker engine.
Don't blame me if you choose local and Claude `rm -rf`s your machine.

### 3. Add a git repository

Open the frontend at http://localhost:8080/ and click "Create Issue".
Tell the agent "add the git repo (git url)  ".
The agent will register the repo in the system and additionally work on a Dockerfile with the dependencies your repo needs.
The agent will also set up a github action to publish the image, configure your git repo to use the image, and then validate that the image has everything you need.

You can repeat this step anytime to register additional git repositories.
You can see what repositories are currently configured on the Settings page.

### 4. Start Working

Simply click "Create Issue" and describe what you want done.
The agents will automatically break down your issue into subtasks, identify the right repositories for each one, make changes and submit PRs.
When agents have work for you to review, they'll make issues assigned to you.

## Core Concepts

A core design principle of Hydra is that *agents and humans have equal access*.
All of the functionality described below is available to your agents.

### Issues

All work in Hydra is represented by issues. Issues can be assigned to either agents or users. 
Issues have a status, which is one of:

- `Open` -- work has not started
- `InProgress` -- work has started
- `Closed` -- work has completed successfully. This status also means "yes/accept" for any approvals escalated to you.
- `Failed` -- work has completed unsuccessfully. This status also means "no/reject" for any approvals escalated to you.
- `Dropped` -- work is no longer required. Use this status to flag issues that do not need to be completed.
- `Rejected` -- work is still required, but the approach is wrong. Use this status to trigger replanning.

Issues also have a progress field which agents automatically update with the current status of the work.
If you manually update the status of an issue, you should also update the progress with commentary.
For example, if you set the status to Rejected, explain in the progress field what the agent should do differently.

Issues have child-of / blocked-on relationships between them. Hydra uses these to automatically spawn agents for issues
that are ready for work.

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
| `hydra-web` | React 19 frontend with a dark terminal-inspired UI. A pnpm monorepo containing a typed API client (`@hydra/api`), component library (`@hydra/ui`), and the SPA (`@hydra/web`). |
<!-- workflow test v5 -->
