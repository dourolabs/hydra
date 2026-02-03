# dourolabs/metis repository index

## Purpose
Metis is an AI-orchestrator: a Rust CLI drives an Axum-based control plane that schedules autonomous jobs onto Kubernetes worker pods. The CLI (`metis`) is the human entry point; `metis-server` stores job state, coordinates background agents, and launches workers.

## Workspace layout (Cargo)
- `metis/`: CLI application and subcommands.
- `metis-server/`: Axum API plus background agents and Kubernetes job engine.
- `metis-common/`: Shared models, API types, and constants used across the workspace.
- `metis-build-cache/`: Build cache utilities for worker builds (workspace crate).
- `metis-s3/`: S3-related helpers (workspace crate).
- `metis-ui/`: Web UI (workspace crate).
- `metis-component-library/`: UI component library (workspace crate).

## Key repo directories
- `images/`: Dockerfiles for server and worker images.
- `scripts/`: Helper scripts (cluster bootstrap, Docker builds, service management).
- `config.toml.sample` files: Template configs; copy to `config.toml` per crate for overrides.

## AGENTS / README / DESIGN notes
- `AGENTS.md`: Source of truth for build/test commands, coding style, and CLI subcommand conventions (subcommands live under `metis/src/command`). Requires `cargo fmt`, `cargo clippy`, and `cargo test` before finishing tasks.
- `metis-server/AGENTS.md`: Route and background module layout rules, plus logging requirements for HTTP handlers and background jobs.
- `README.md`: Top-level overview, repository layout, prerequisites, configuration, and local development guidance (Postgres, GitHub App, kind cluster workflows).
- `DESIGN.md`: System design and issue-tracker workflow (statuses, readiness rules, agent lifecycle, branch tracking strategy).

## Build / test commands (per AGENTS.md)
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Run / dev commands (README highlights)
- `cargo run -p metis -- jobs list` (CLI against a server)
- `METIS_CONFIG=metis-server/config.toml cargo run -p metis-server` (server)
- `./scripts/docker-build.sh` (build deployment containers)
- `./scripts/dev-postgres.sh start` (local Postgres)
- `./scripts/service.sh start` (kind cluster dev service)
