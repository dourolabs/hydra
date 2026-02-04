# dourolabs/metis
hello world
  __  __ _______ _____
 |  \/  |__   __|_   _|
 | \  / |  | |    | |
 | |\/| |  | |    | |
 | |  | |  | |   _| |_
 |_|  |_|  |_|  |_____|

## Overview
Metis is an experimental AI orchestration system: a Rust CLI (`metis`) drives an Axum-based control plane (`metis-server`) that schedules autonomous jobs onto Kubernetes worker pods. The CLI is the human interface (issues, patches, jobs, logs, chat), while the server stores job state, runs background agents, and launches workers.

## Repository structure (key modules)
- `metis/`: CLI crate. Subcommands live under `metis/src/command` (thin sync wrappers around async helpers).
- `metis-server/`: Axum HTTP API plus background workers and Kubernetes job engine; routes under `metis-server/src/routes`, background logic under `metis-server/src/background` and `metis-server/src/job_engine`, shared state under `metis-server/src/store`.
- `metis-common/`: Shared models and API types (`MetisId`, job/log/issue/patch structs, env var constants) used by both crates.
- `images/`: Dockerfiles for server and worker images.
- `scripts/`: Automation scripts (Docker builds, cluster/service helpers, dev Postgres).
- `metis-ui/`, `metis-component-library/`, `metis-s3/`, `metis-build-cache/`: Supporting UI/services and build cache components.

## Build & test
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo test --workspace`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`

## Notable docs
- `README.md`: High-level overview, layout, configuration, and local development workflows.
- `AGENTS.md`: Repository conventions, required commands, and contribution expectations.
- `metis-server/AGENTS.md`: API route/background layout and logging policy.
- `GETTING_STARTED.md`: Setup guide for local development.
- `DESIGN.md`: Architectural and product design notes.
