# dourolabs/metis Repository Summary

## Overview
Metis is an AI-orchestrator: a Rust CLI (`metis`) drives an Axum-based control plane (`metis-server`) that schedules autonomous jobs onto Kubernetes worker pods. Shared models live in `metis-common`. The repo also includes a UI (`metis-ui`) and a component library (`metis-component-library`).

## Repository Structure
- `metis/`: CLI crate with subcommands under `metis/src/command` and client helpers in `metis/src/client`.
- `metis-server/`: Axum API + background agents + Kubernetes job engine.
- `metis-common/`: Shared models/types (IDs, API schemas, constants) used by CLI/server.
- `metis-ui/`: UI crate (Rust frontend) with assets.
- `metis-component-library/`: Shared UI components.
- `metis-s3/`: Auxiliary crate for S3-related integration.
- `images/`: Dockerfiles for server/worker images.
- `scripts/`: Automation scripts (Docker builds, cluster/service helpers).
- `config.toml.sample` files: templates to copy to `config.toml` per crate.

## Key Docs
- `README.md`: High-level overview, repo layout, prerequisites, and local/dev setup (including kind cluster workflow).
- `GETTING_STARTED.md`: Quickstart for cloning, building the CLI, connecting to a server, and using issues/patches/documents workflows.
- `DESIGN.md`: System design (issue graph, readiness rules, agent lifecycle, git tracking branches).
- `metis/docs/issues.md`: CLI issue management flows (list/describe/update/todo).
- `metis/docs/patches.md`: Patch workflow and CLI usage.
- `metis/docs/documents.md`: Document storage workflow via the CLI.
- `AGENTS.md`: Repo-wide conventions and required commands; `metis-server/AGENTS.md` adds server-specific routing/logging rules.

## Notable Modules (PM Workflow Oriented)
- Issue tracking and coordination:
  - CLI entrypoints in `metis/src/command` for `issues`, `patches`, `documents`, etc.
  - Shared issue/patch types in `metis-common/src/models` and API schemas in `metis-common/src/api`.
  - Server routes in `metis-server/src/routes` (resource-based modules like `issues`, `patches`, `repos`).
- Job orchestration:
  - `metis-server/src/job_engine`: Kubernetes job creation and lifecycle handling.
  - `metis-server/src/background`: background workers and schedulers for agents.
  - `metis-server/src/store`: persistence and in-memory state helpers.
- Auth and GitHub integration:
  - CLI helpers in `metis/src/git.rs` and GitHub device flow in `metis/src/github_device_flow.rs`.
  - Shared GitHub types in `metis-common/src/github.rs`.
- UI/dashboard:
  - `metis-ui/` hosts the dashboard for issues/jobs/patches.
  - `metis-component-library/` provides shared UI components.

## AGENTS.md Highlights (Operational Expectations)
- Commands required before finishing tasks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- CLI subcommands live in `metis/src/command` with thin sync wrappers over async helpers.
- Use `MetisId` for identifiers; avoid shelling out to `git` (use libgit2).
- CLI env vars should be declared on arg structs via `#[arg(env = ...)]`.
- `metis-server/AGENTS.md`: route modules under `metis-server/src/routes`; background jobs under `metis-server/src/background` and `metis-server/src/job_engine`; log every HTTP handler and background job at `info!`.
