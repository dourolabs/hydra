# dourolabs/metis repository summary

## Overview
Metis is an agent coordination framework: a Rust CLI (`metis`) talks to an Axum-based control plane (`metis-server`) that schedules AI coding jobs onto Kubernetes worker pods. Shared models and API types live in `metis-common`, with additional crates for build caching (`metis-build-cache`), S3-backed artifact storage (`metis-s3`), and UI components (`metis-ui`, `metis-component-library`).

## Workspace layout
- `metis/`: CLI crate with subcommands under `metis/src/command` (issues, patches, jobs, dashboard, chat, repos, etc.).
- `metis-server/`: Axum HTTP API, background workers, Kubernetes job engine, and Postgres/memory stores.
- `metis-common/`: Shared identifiers (`MetisId`), models, API request/response types, and constants.
- `metis-build-cache/`: Git + storage-backed build cache client/server helpers.
- `metis-s3/`: S3-compatible artifact store service.
- `metis-ui/` + `metis-component-library/`: UI frontend and reusable components/styles.
- `images/`: Dockerfiles for server, worker, UI, component library, and S3 service.
- `scripts/`: Local dev helpers (docker build, service bootstrap, postgres, worker entrypoint).

## Key technical notes
- CLI interacts with the server for job orchestration, issue/patch tracking, and logs.
- Server owns job scheduling, background polling (e.g., GitHub), and persistence; supports Kubernetes job engine.
- Configuration is via `config.toml.sample` copies per crate, plus env vars (see `README.md`).
- Metis uses an issue/patch workflow where agents update issue status and create patches.

## Configuration & runtime
- CLI: `metis/config.toml.sample` and `METIS_SERVER_URL` (or `--server-url`).
- Server: `metis-server/config.toml.sample` plus `METIS_CONFIG=...` when running.
- Build cache: `metis-build-cache` crate configuration in `metis-build-cache/src/config.rs`.
- S3: `metis-s3/config.toml.sample`.

## Build and test
- `cargo check --workspace`
- `cargo build --workspace --all-targets`
- `cargo test --workspace`
- Optional Postgres integration tests via `./scripts/dev-postgres.sh` and `DATABASE_URL`.

## Notable docs
- `README.md`: High-level overview, configuration, and local dev workflows.
- `DESIGN.md`: System motivation, issue workflow, and git state management.
- `AGENTS.md` + crate-specific `AGENTS.md`: repo and subproject conventions.

## Testing highlights
- Extensive CLI/server integration tests under `metis/tests` and `metis-server/src/test`.
- Build cache integration tests under `metis-build-cache/tests`.

