# dourolabs/metis repo index

## Architecture summary
Metis is an agent coordination framework built around a Rust CLI and an Axum-based control plane. The CLI (`metis`) is the primary user interface for humans and agents. The server (`metis-server`) persists state, exposes HTTP APIs, and schedules background workers that launch Kubernetes jobs. Shared types live in `metis-common` to keep server and client in sync. Supporting services include `metis-s3` (minimal S3-compatible storage) and `metis-build-cache` (build artifact cache).

Key ideas:
- Work is tracked as issues with status + dependency graph; agents and humans use the same CLI.
- Background workers in the server poll queues and spawn Kubernetes jobs for ready issues.
- Git state is managed via dedicated tracking branches per issue/task.

## Key paths and module layout

### CLI (`metis`)
- `metis/src/main.rs`: CLI entrypoint.
- `metis/src/command/*`: Each CLI subcommand in its own file or module.
- `metis/src/client/*`: HTTP client helpers.
- `metis/src/git.rs`: libgit2 helpers for CLI git operations.
- `metis/docs/*`: CLI-specific docs (issues, patches, documents).
- `metis/config.toml.sample`: Sample configuration.

### Server (`metis-server`)
- `metis-server/src/main.rs`: Server entrypoint.
- `metis-server/src/routes/*`: Axum route handlers per resource (jobs, issues, patches, etc.).
- `metis-server/src/domain/*`: Domain structs mapped to API types.
- `metis-server/src/app/*`: AppState, validation, and request coordination.
- `metis-server/src/background/*`: Background workers and schedulers.
- `metis-server/src/job_engine/*`: Kubernetes job orchestration.
- `metis-server/src/store/*`: Data stores (memory/postgres) and issue graph.
- `metis-server/src/merge_queue/*`: Merge queue logic.
- `metis-server/migrations/*`: Postgres schema migrations.
- `metis-server/config.toml.sample`: Sample configuration.

### Shared types (`metis-common`)
- `metis-common/src/api/v1/*`: Versioned API wire types.
- `metis-common/src/models/*`: Shared models such as IDs, reviews, activity logs.
- `metis-common/src/ids.rs`: `MetisId` alias and ID helpers.

### Build cache + storage
- `metis-build-cache/src/*`: Build cache client/storage logic.
- `metis-s3/src/*`: Minimal S3-compatible service.

### UI (optional/ancillary)
- `metis-ui/src/*`: Rust-based UI app.
- `metis-component-library/src/*`: Shared UI components/styles.

### Infrastructure + scripts
- `images/*.Dockerfile`: Container images (server, worker, UI, S3).
- `scripts/*`: Helper scripts (docker builds, dev postgres, service management).

## Ownership boundaries and responsibilities
- `metis` (CLI): User and agent interface; submits issues, patches, jobs; should remain thin wrappers around async helpers.
- `metis-server` (control plane): Auth, persistence, scheduling, Kubernetes job lifecycle, background agents.
- `metis-common` (contracts): API and model types shared across services; must be additive and forward compatible.
- `metis-build-cache` + `metis-s3`: Artifact cache layer and storage service; config must be explicit, no env defaults embedded.
- UI crates (`metis-ui`, `metis-component-library`): Optional front-end and component library.

## Reference docs
- `README.md`: Top-level overview, setup, and local dev instructions.
- `DESIGN.md`: System design, issue lifecycle, and git state management.
- `AGENTS.md`: Repo-wide dev guidelines (build/test, coding conventions).
- `metis-server/AGENTS.md`, `metis-common/AGENTS.md`, `metis-build-cache/AGENTS.md`: Module-specific expectations.
