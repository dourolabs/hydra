# dourolabs/metis repository index

## Summary
Metis is an AI-orchestrator built around a Rust CLI (`metis`) and an Axum-based control plane (`metis-server`) that schedule autonomous jobs onto Kubernetes worker pods. Shared models and API types live in `metis-common`. The repo also includes supporting services (S3-compatible store, build cache) plus UI crates and deployment scripts.

## Key docs
- `AGENTS.md`: workspace conventions, build/test commands, and PR expectations.
- `DESIGN.md`: architecture and issue/agent workflow design rationale.
- `GETTING_STARTED.md`: setup walkthrough for CLI usage and local development.
- `README.md`: high-level overview and repository layout.
- `metis/docs/issues.md`, `metis/docs/patches.md`, `metis/docs/documents.md`: CLI workflows for issue/patch/document management.
- `metis-server/AGENTS.md`: route/background job layout guidance and logging expectations.

## Top-level structure
- `metis/`: Rust CLI crate. Entry point: `metis/src/main.rs`. Subcommands live under `metis/src/command/`.
- `metis-server/`: Axum API + background workers. Entry point: `metis-server/src/main.rs`. Routes in `metis-server/src/routes/`, background jobs in `metis-server/src/background/` and `metis-server/src/job_engine/`.
- `metis-common/`: Shared API types/models and IDs (`metis-common/src/lib.rs`).
- `metis-build-cache/`: Build cache service crate with tests under `metis-build-cache/tests/`.
- `metis-s3/`: Minimal S3-compatible service; see `metis-s3/README.md` for config and Docker usage.
- `metis-ui/`: UI crate and assets (Fly deployment config in `fly.toml`).
- `metis-component-library/`: Shared UI components and assets (also Fly config).
- `images/`: Dockerfiles for server, worker, UI, component library, and S3 service.
- `scripts/`: Helper scripts for Docker builds, local Postgres, service orchestration, and worker entrypoint.
- `config.toml.sample` files: per-crate configuration templates (copy to `config.toml`).

## Notable entrypoints
- CLI: `metis/src/main.rs`
- CLI command implementations: `metis/src/command/`
- Server: `metis-server/src/main.rs`, with app state in `metis-server/src/app/`
- Server routes: `metis-server/src/routes/`
- Server background workers/job engine: `metis-server/src/background/`, `metis-server/src/job_engine/`
- Shared API models: `metis-common/src/api/`
