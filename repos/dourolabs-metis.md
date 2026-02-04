# dourolabs/metis repo summary

## Purpose
Metis is an AI-orchestrator: a Rust CLI (`metis`) talks to an Axum-based control plane (`metis-server`) that schedules agent jobs onto Kubernetes worker pods.

## Structure and key modules
- `metis/` (CLI): End-user CLI with subcommands (spawn, jobs, logs, patches, issues, chat, dashboard). Subcommands live under `metis/src/command`.
- `metis-server/` (API + workers): Axum HTTP API plus background agents and Kubernetes job engine; owns orchestration, persistence, and webhooks.
- `metis-common/` (shared models): Common types like `MetisId`, job/log/issue/patch models, and shared constants.
- `metis-ui/` + `metis-component-library/`: Frontend dashboard and shared UI components.
- `metis-s3/`: S3-related support crate.
- `images/`: Dockerfiles for server/worker images.
- `scripts/`: Automation (Docker build, kind cluster helpers, Postgres dev).

## Build, test, and lint commands
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo test --workspace`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`

## Notable docs
- `README.md`: High-level overview, layout, config, and local dev.
- `AGENTS.md`: Coding conventions, testing expectations, and CLI patterns.
- `metis-server/AGENTS.md`: Server route/background worker conventions.
- `GETTING_STARTED.md`: Quickstart for building and using the CLI/dashboard.
- `DESIGN.md`: System design and issue/agent lifecycle.
- `metis/docs/issues.md`, `metis/docs/patches.md`, `metis/docs/documents.md`: CLI reference for issues, patches, and documents.
