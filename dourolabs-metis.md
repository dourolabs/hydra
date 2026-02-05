# dourolabs/metis architecture notes

## Overview
Metis is an agent coordination framework. A Rust CLI (`metis`) talks to an Axum-based control plane (`metis-server`) that schedules autonomous jobs onto Kubernetes worker pods. Work is tracked as issues and patches in the Metis issue tracker, and background agents can spawn/close tasks based on issue readiness.

Design points pulled from repo docs:
- Issues have explicit statuses (Open/InProgress/Dropped/Closed) plus inferred readiness. The system spawns agents for Ready issues assigned to AI agents.
- Workers use tracking branches for issue/task base/head (`metis/<issue-id>/base`, `metis/<issue-id>/head`, `metis/<task-id>/base`, `metis/<task-id>/head`).

## Workspace modules
Declared in `Cargo.toml` workspace:
- `metis` (CLI)
- `metis-server` (Axum API + background workers)
- `metis-common` (shared models / API types)
- `metis-build-cache` (build cache service)
- `metis-s3` (minimal S3-compatible service for build cache)
- `metis-component-library` (UI component crate)
- `metis-ui` (UI app crate)

## Key paths
Top-level:
- `README.md`: repo layout, local dev, server/CLI configuration.
- `DESIGN.md`: system design, issue lifecycle, and git tracking branches.
- `GETTING_STARTED.md`: onboarding and environment setup.
- `images/`: Dockerfiles for server/worker and supporting services.
- `scripts/`: cluster bootstrap, Docker build, and service management helpers.
- `config.toml.sample` files per crate: copy to `config.toml` for overrides.

CLI (`metis`):
- `metis/src/command`: one file per subcommand.
- `metis/src/constants.rs`: centralized CLI constants.

Server (`metis-server`):
- `metis-server/src/routes`: Axum handlers per resource (jobs, issues, repos, etc.).
- `metis-server/src/background`: background agents/queues.
- `metis-server/src/job_engine`: per-job entrypoints.
- `metis-server/src/store`: in-memory store + shared state helpers.
- `metis-server/src/domain`: domain structs mapped to API types.

Shared (`metis-common`):
- API v1 types are wire contracts and must be additive-only.

Build cache / storage:
- `metis-build-cache/`: build cache service (must be configured explicitly, no env defaults).
- `metis-s3/`: S3-compatible store used by build cache (filesystem-backed).

UI:
- `metis-ui/` and `metis-component-library/`: UI crates (Rust). Treat as a separate ownership boundary from CLI/server.

Docs:
- `metis/docs/issues.md`, `metis/docs/patches.md`, `metis/docs/documents.md`: CLI subcommand behavior for issues/patches/documents.

## Ownership boundaries & invariants
- `metis-common` owns API contract types. Changes must be additive; server/domain structs must stay in sync with API v1 types.
- `metis` owns CLI UX and libgit2 interactions; avoid shelling out to git.
- `metis-server` owns lifecycle rules, validation, and orchestration logic. Store layer should persist/index without enforcing app-level transitions.
- `metis-build-cache` and `metis-s3` are supporting services; configuration is explicit and should be passed in (no env defaults in build-cache code).
- `metis-ui` / `metis-component-library` are UI-specific ownership areas and should be documented separately if UI changes are requested.

## Notes for contributors
- See `AGENTS.md` for repo-wide coding/testing rules.
- Additional AGENTS: `metis/AGENTS.md`, `metis-server/AGENTS.md`, `metis-common/AGENTS.md`, `metis-build-cache/AGENTS.md`.
