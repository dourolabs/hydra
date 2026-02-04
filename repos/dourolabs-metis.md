# dourolabs/metis repository inventory

## Top-level docs
- `AGENTS.md`: repo-wide contribution rules, build/test commands, coding style, and PR/patch workflow (see sub-AGENTS below for module-specific rules).
- `README.md`: overview, repository layout, config, and local dev workflows for the CLI/server.
- `GETTING_STARTED.md`: onboarding steps and quickstart guidance.
- `DESIGN.md`: architecture and design notes.
- `CLAUDE.md`: auxiliary agent instructions.

## Workspace crates (Cargo workspace)
- `metis/`: CLI crate (`metis`) with subcommands under `metis/src/command`.
- `metis-server/`: Axum API + background workers and job orchestration.
- `metis-common/`: shared models, API types, and identifiers used by CLI/server.
- `metis-s3/`: minimal S3-compatible service backing local build cache (see `metis-s3/README.md`).

## Frontend
- `metis-ui/`: end-user UI frontend.
- `metis-component-library/`: shared UI components.

## Supporting services
- `metis-build-cache/`: build cache service; used by workers/infra.

## Infrastructure & automation
- `images/`: Dockerfiles for server/worker/aux services.
- `scripts/`: automation scripts (Docker builds, service lifecycle, dev helpers).

## Other top-level items
- `Cargo.toml`, `Cargo.lock`: workspace manifest and lockfile.
- `flake.nix`, `flake.lock`: Nix-based dev tooling.
- `notes.txt`: misc notes.

## Sub-AGENTS.md references
- `metis/AGENTS.md`: CLI-specific rules (per-command files, constants, client compatibility tests).
- `metis-server/AGENTS.md`: route/background layout, logging, domain/API mapping guidance.
- `metis-common/AGENTS.md`: API v1 compatibility constraints.
- `metis-build-cache/AGENTS.md`: build-cache config rules.

Hello world
Hola mundo
你好，世界
