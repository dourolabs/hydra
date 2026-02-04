# dourolabs/metis repository index

## Overview
Metis is an AI-orchestrator: a Rust CLI drives an Axum-based control plane that schedules autonomous jobs onto Kubernetes worker pods. The CLI (`metis`) is the human interface; `metis-server` stores job state, coordinates background agents, and launches workers. See `README.md` for setup, and `DESIGN.md` for system concepts.

## Key docs
- `README.md`: high-level orientation, setup, configuration, and local dev workflows.
- `DESIGN.md`: system motivation, issue/agent orchestration model, and Git branch tracking design.
- `AGENTS.md`: repository-wide conventions (commands, style, testing, PR expectations).
- `metis-server/AGENTS.md`: route/background module layout + logging requirements.
- `GETTING_STARTED.md`: onboarding / quickstart steps (see file for walkthrough).

## Top-level layout
- `metis/`: CLI crate. Entry point `metis/src/main.rs`; subcommands live under `metis/src/command`.
- `metis-server/`: Axum API + background workers. Entry point `metis-server/src/main.rs`; routes in `metis-server/src/routes`; background workers in `metis-server/src/background` and `metis-server/src/job_engine`.
- `metis-common/`: shared models, IDs, API types, constants (`metis-common/src/lib.rs`).
- `metis-ui/`: UI crate (assets in `metis-ui/assets`, app code in `metis-ui/src`).
- `metis-component-library/`: shared UI components (assets + `src`).
- `metis-s3/`: S3 helper/service crate (see `metis-s3/README.md`).
- `metis-build-cache/`: build cache service crate with tests.
- `images/`: Dockerfiles for server/worker images.
- `scripts/`: automation (docker builds, cluster setup, dev Postgres, service management).
- `notes.txt`, `CLAUDE.md`, `flake.nix`/`flake.lock`: local tooling / Nix shell configuration.

## Notable entrypoints
- CLI: `metis/src/main.rs` + `metis/src/command/*` per subcommand.
- Server: `metis-server/src/main.rs` + `metis-server/src/routes/*` for HTTP handlers.
- Background jobs: `metis-server/src/job_engine/*` and `metis-server/src/background/*`.
- Shared API/types: `metis-common/src/api` + `metis-common/src/models`.

## Config samples
- `metis/config.toml.sample`
- `metis-server/config.toml.sample`
- `metis-s3/config.toml.sample`

