# dourolabs/metis Repository Index

## Overview
Rust workspace powering the Metis CLI, API server, UI, and supporting services. Core crates live at the repo root with shared types in `metis-common`.

## Workspace Crates & Services
- `metis/`: CLI for issues, patches, jobs, agents, repos, caches, users, and documents.
- `metis-server/`: Axum HTTP API plus background workers and job engine.
- `metis-common/`: Shared models, API v1 contracts, IDs, constants, utilities.
- `metis-build-cache/`: Build cache client + storage abstractions.
- `metis-s3/`: S3-sidecar service for object storage interactions.
- `metis-ui/`: UI frontend (Rust-based) with assets in `metis-ui/assets`.
- `metis-component-library/`: Shared UI component library + styles.

## Key Entry Points
- CLI binary: `metis/src/main.rs`
- CLI lib: `metis/src/lib.rs`
- API server binary: `metis-server/src/main.rs`
- API server lib: `metis-server/src/lib.rs`
- Build cache lib: `metis-build-cache/src/lib.rs`
- S3 service binary: `metis-s3/src/main.rs`
- UI binary: `metis-ui/src/main.rs`
- Component library binary: `metis-component-library/src/main.rs`

## Notable Modules
- CLI commands: `metis/src/command/` (subcommands per file)
- CLI git integration: `metis/src/git.rs`
- API routes: `metis-server/src/routes/`
- API domain models: `metis-server/src/domain/`
- Background jobs: `metis-server/src/background/`
- Job engine: `metis-server/src/job_engine/`
- Stores: `metis-server/src/store/`
- Shared API contracts: `metis-common/src/api/v1/`
- Shared IDs/constants: `metis-common/src/ids.rs`, `metis-common/src/constants.rs`

## Configuration & Samples
- `metis/config.toml.sample`
- `metis-server/config.toml.sample`
- `metis-s3/config.toml.sample`

## Notable Docs
- `README.md`: high-level repo overview
- `GETTING_STARTED.md`: setup guidance
- `DESIGN.md`: architecture notes
- `metis/docs/issues.md`: CLI issue flows
- `metis/docs/patches.md`: CLI patch flows
- `metis/docs/documents.md`: CLI document flows
- `metis-s3/README.md`: S3 service usage
- `AGENTS.md`, `metis-server/AGENTS.md`, `metis-common/AGENTS.md`: local contribution rules

## Build & Test Commands
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo run -p metis -- jobs list`
- `METIS_CONFIG=metis-server/config.toml cargo run -p metis-server`
- `./scripts/docker-build.sh`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Images and Scripts
- Dockerfiles: `images/`
- Helper scripts: `scripts/`

hello world
