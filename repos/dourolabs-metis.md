# dourolabs/metis repository index

## Overview
Metis is an agent coordination framework that pairs a Rust CLI (metis) with an Axum-based control plane (metis-server) to schedule autonomous jobs onto Kubernetes worker pods. The CLI is how humans interact with jobs, logs, issues, and patches; the server stores job state, runs background agents, and launches workers via Kubernetes.

Key docs:
- `README.md`: high-level overview, repo layout, prerequisites, local dev workflows.
- `AGENTS.md`: coding conventions, build/test commands, PR expectations.
- `DESIGN.md`: system architecture, issue lifecycle, and agent coordination model.
- `GETTING_STARTED.md`: end-to-end walkthrough for installing, connecting to a server, and creating issues.
- `metis-server/AGENTS.md`, `metis/AGENTS.md`, `metis-common/AGENTS.md`: per-crate conventions.
- `metis-build-cache/AGENTS.md`: build-cache configuration constraints.
- `metis/docs/issues.md`, `metis/docs/patches.md`, `metis/docs/documents.md`: CLI workflow guides.
- `metis-s3/README.md`: local/dev usage of the auxiliary S3-compatible service.

## Repository layout (top level)
- `metis/`: CLI crate; subcommands live in `metis/src/command`.
- `metis-server/`: Axum API, background workers, Kubernetes job engine.
- `metis-common/`: shared models and API types.
- `metis-build-cache/`: build cache client + helpers (used by workers).
- `metis-s3/`: minimal S3-compatible service backed by the local filesystem.
- `metis-ui/`, `metis-component-library/`: UI crates (Rust-based) with assets and Fly.io config.
- `images/`: Dockerfiles for server and worker images.
- `scripts/`: automation for Docker builds, cluster bootstrap, service lifecycle, and local Postgres.
- `config.toml.sample` files: per-crate config templates to copy to `config.toml`.

## Workspace crates and module structure
- `metis` (CLI): each command is its own module under `metis/src/command`. CLI constants live in `metis/src/constants.rs`. Prefer thin sync wrappers around async helpers.
- `metis-server` (API + background): HTTP handlers live in `metis-server/src/routes` (one file per resource), background job logic in `metis-server/src/job_engine` and `metis-server/src/background`, shared state in `metis-server/src/store`. Domain structs in `metis-server/src/domain` map to `metis-common::api::v1` types.
- `metis-common` (shared models): API v1 types are the wire contract; changes must be additive and mirrored in server domain structs and conversion impls.
- `metis-build-cache`: crate for build-cache operations; do not embed env var defaults (see its AGENTS).
- `metis-s3`: local S3-compatible service used for build-cache development/testing.
- `metis-ui` / `metis-component-library`: Rust UI crates with shared components and assets.

## Configuration and runtime
- Copy `config.toml.sample` to `config.toml` per crate to override defaults.
- Server launches with `METIS_CONFIG=metis-server/config.toml cargo run -p metis-server`.
- CLI defaults to `~/metis/config.toml` unless `--config` is provided; can also use `METIS_SERVER_URL`.

## Build, test, and development commands
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo test --workspace`
- `./scripts/docker-build.sh` to build deployment containers.

## Conventions and requirements
- Use `MetisId` for identifiers instead of raw `String`.
- CLI git operations must use libgit2, not shelling out to git.
- When CLI commands need environment variables, declare them on the arg struct (`#[arg(env = ...)]`).
- Public docs: avoid adding CLI command details to `README.md` unless requested.
- Logging: routes and background jobs in `metis-server` must emit `info!` logs with key identifiers and outcomes.
- Testing checklist (per repo guidelines): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
