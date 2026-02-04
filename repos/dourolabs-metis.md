# dourolabs/metis repo summary

## Overview
Metis is an agent coordination framework / AI orchestrator. A Rust CLI (`metis`) talks to an Axum-based control plane (`metis-server`) that schedules agent jobs on Kubernetes. The server stores job state, runs background workers, and launches worker pods, while the CLI is the human interface for spawning work, tailing logs, and managing issues/patches. Design notes are in `DESIGN.md`, with repo conventions in `AGENTS.md`.

## Top-level layout
- `metis/`: CLI crate; subcommands live under `metis/src/command`.
- `metis-server/`: Axum HTTP API plus background agents and Kubernetes job engine.
- `metis-common/`: Shared models, types, and constants (`MetisId`, job/log/issue/patch types).
- `metis-build-cache/`: Build cache crate for worker builds.
- `metis-s3/`: S3 integration/service crate.
- `metis-ui/`: UI crate.
- `metis-component-library/`: Component library crate.
- `images/`: Dockerfiles for server/worker images.
- `scripts/`: Automation (docker builds, service management, dev Postgres, etc.).
- `DESIGN.md`, `README.md`, `GETTING_STARTED.md`, `AGENTS.md`: design, onboarding, and repo conventions.

## Entrypoints
- CLI: `metis/src/main.rs`.
- Server: `metis-server/src/main.rs`.
- Other binaries: `metis-s3/src/main.rs`, `metis-ui/src/main.rs`, `metis-component-library/src/main.rs`.

## Build & test
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo test --workspace`
- Required before submit: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- Postgres-backed tests are ignored by default; README documents running them with `DATABASE_URL` and `--include-ignored`.

## Configuration
- Sample configs: `metis/config.toml.sample`, `metis-server/config.toml.sample`, `metis-s3/config.toml.sample`.
- CLI config: `--config <file>` or `~/metis/config.toml`; server URL via `[server].url`, `METIS_SERVER_URL`, or `--server-url`.
- Server config: `METIS_CONFIG=metis-server/config.toml` and environment variables like `OPENAI_API_KEY`.

## Notes
- Docker builds: `./scripts/docker-build.sh`.
- Local Postgres helper: `./scripts/dev-postgres.sh`.
- Kind-based local deployment: `./scripts/service.sh`.
