# dourolabs/metis - Repository Index

## Overview

Metis is an AI agent coordination framework built in Rust. It orchestrates multiple simultaneous AI coding agents through an issue tracker model where both humans and AI agents interact via the same CLI. The system schedules autonomous jobs onto Kubernetes worker pods, manages git state across agent sessions, and coordinates work through issue dependency graphs.

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `metis` | End-user CLI for interacting with the system: spawning jobs, tailing logs, managing issues/patches, running a TUI dashboard, and interactive AI chat. |
| `metis-server` | Axum-based HTTP API server with background workers for job scheduling, Kubernetes orchestration, GitHub PR polling, and data persistence (in-memory or PostgreSQL). |
| `metis-common` | Shared models, API v1 wire types, type-safe ID system (`MetisId`), constants, and versioning used by both CLI and server. |
| `metis-build-cache` | Build artifact caching with S3 backend, tar/zstd compression, and git-aware cache key generation. |
| `metis-s3` | S3-compatible HTTP service for storing and retrieving build artifacts, built on Axum. |
| `metis-component-library` | Reusable Dioxus UI components (Button, Input, ToggleSwitch, Select) with SCSS styling. |
| `metis-ui` | Main Dioxus web interface for the Metis dashboard, deployed via Fly.io. |

## Key API Areas

The `metis-server` exposes an HTTP API with routes for:

- **Jobs** (`/v1/jobs/`) - Create, list, kill, stream logs, query status of AI agent jobs
- **Issues** (`/v1/issues/`) - CRUD for work items with status tracking, dependency graphs, todo lists, and version history
- **Patches** (`/v1/patches/`) - Manage code patches/PRs with GitHub integration, review tracking, and asset uploads
- **Documents** (`/v1/documents/`) - CRUD for markdown documents in the Metis document store
- **Agents** (`/v1/agents/`) - Agent listing and management
- **Repositories** (`/v1/repositories/`) - Repository metadata CRUD
- **Users** (`/v1/users/`) - User listing and authentication
- **Merge Queues** (`/v1/merge_queues/`) - Automated merge queue management
- **Auth/Login** - OAuth login flow and token-based authentication

## Key Domain Types

| Type | Module | Description |
|------|--------|-------------|
| `Task` | `metis-common::api::v1::jobs` | An AI agent job with prompt, git context (`BundleSpec`), container image, model selection, resource limits, status, and error tracking. |
| `Issue` | `metis-common::api::v1::issues` | A work item with description, type (Bug/Feature/Task/Chore/MergeRequest), status (Open/InProgress/Closed/Dropped), assignee, progress notes, dependencies, and todo list. |
| `Patch` | `metis-common::api::v1::patches` | A code change (maps to GitHub PR) with status (Open/Closed/Merged/ChangesRequested), review tracking, and asset attachments. |
| `Document` | `metis-common::api::v1::documents` | A markdown document with title, body, optional path, and creator tracking. |
| `MetisId` | `metis-common::ids` | Type-safe ID system with prefixed variants: `IssueId` (i-), `PatchId` (p-), `TaskId` (t-), `DocumentId` (d-). |
| `BundleSpec` | `metis-common::api::v1::jobs` | Git context for a job: either `None` or `GitRepository { url, rev }`. |
| `TaskStatusLog` | `metis-common::api::v1::task_status` | Timeline of task state transitions derived from task version history. |

## Key Directories

| Path | Purpose |
|------|---------|
| `metis/src/command/` | CLI subcommand implementations: `jobs/`, `issues.rs`, `patches.rs`, `documents.rs`, `agents.rs`, `dashboard.rs`, `chat.rs`, `repos.rs`, `users.rs`, `caches.rs`, `output.rs` |
| `metis-server/src/routes/` | HTTP API handlers, one file per resource (plus `jobs/` subdirectory for job-specific routes) |
| `metis-server/src/background/` | Background workers: `scheduler.rs`, `spawner.rs`, `process_pending_jobs.rs`, `monitor_running_jobs.rs`, `poll_github_patches.rs`, `run_spawners.rs` |
| `metis-server/src/domain/` | Server-side business logic models: `issues.rs`, `jobs.rs`, `patches.rs`, `task_status.rs`, `actors.rs`, `users.rs`, `documents.rs` |
| `metis-server/src/store/` | Data persistence layer with `Store` trait, `memory_store.rs`, `postgres.rs` (v1 JSONB), `postgres_v2.rs` (columnar), `migration.rs`, `issue_graph.rs` |
| `metis-server/src/job_engine/` | Kubernetes job execution engine |
| `metis-common/src/api/v1/` | API v1 wire types (request/response structs) shared between CLI and server |
| `metis-common/src/ids.rs` | MetisId type system with prefixed ID variants |
| `images/` | Dockerfiles for server, worker, S3 service, UI, and component library |
| `scripts/` | Deployment automation: `service.sh` (Kubernetes), `docker-build.sh`, `dev-postgres.sh`, `worker-entrypoint.sh` |

## Build & Test Commands

```bash
# Compile check
cargo check --workspace

# Full build
cargo build --workspace --all-targets

# Run tests
cargo test --workspace

# Run tests including Postgres-backed store tests (requires local Postgres)
DATABASE_URL=postgres://postgres:postgres@localhost:5432/metis cargo test --workspace --all-targets -- --include-ignored

# Lint
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# Run CLI
cargo run -p metis -- jobs list

# Run server
METIS_CONFIG=metis-server/config.toml cargo run -p metis-server

# Build Docker images (for kind cluster)
./scripts/docker-build.sh

# Local Postgres for development
./scripts/dev-postgres.sh start
```

## Architecture Notes

- **Issue-driven orchestration**: All work is modeled as issues with dependency graphs (blocked-on, child-of). The system infers readiness and spawns AI agents for ready issues.
- **Agent/human equivalence**: Both interact through the same `metis` CLI, enabling delegation of any work (coding, reviewing, planning) to AI agents.
- **Git state management**: Tracking branches (`metis/<issue-id>/head`, `metis/<task-id>/head`) preserve work across agent sessions, enabling multi-session task completion.
- **Pluggable storage**: The `Store` trait abstracts persistence, with in-memory (development) and PostgreSQL v1/v2 (production) backends.
- **Background workers**: A scheduler coordinates job processing, Kubernetes monitoring, GitHub PR polling, and issue/task spawning.