# dourolabs/metis repo index

## Top-level structure
- `AGENTS.md` — repo guidelines, build/test checklist, conventions.
- `README.md` — high-level overview, repo layout, setup, local dev.
- `DESIGN.md` — system design + issue/agent workflow + git state model.
- `GETTING_STARTED.md` — onboarding walkthrough.
- `Cargo.toml` — workspace (Rust) manifest.
- `metis/` — CLI crate.
- `metis-server/` — Axum API + background workers.
- `metis-common/` — shared models/constants/types.
- `metis-ui/` + `metis-component-library/` — frontend UI + shared components.
- `metis-s3/` — auxiliary service crate.
- `images/` — Dockerfiles.
- `scripts/` — automation helpers.

## Key crates and modules

### CLI (`metis/`)
- `metis/src/main.rs` — CLI entrypoint.
- `metis/src/command/` — subcommands (keep each subcommand in its own file).
- `metis/src/client/` — API client glue.
- `metis/src/git.rs` — CLI git operations (libgit2, no shelling out).

### Server (`metis-server/`)
- `metis-server/src/main.rs` — server entrypoint.
- `metis-server/src/app/` — application wiring.
- `metis-server/src/routes/` — Axum HTTP routes.
- `metis-server/src/domain/` — domain models (patches, issues, jobs, etc.).
- `metis-server/src/background/` — background workers (GitHub polling, queues).
- `metis-server/src/store/` — persistence.
- `metis-server/src/job_engine/` — Kubernetes job orchestration.

### Shared (`metis-common/`)
- `metis-common/src/models/` — shared domain models.
- `metis-common/src/api/` — API request/response payloads.
- `metis-common/src/ids.rs` — `MetisId` alias + id helpers.

## Relevant docs
- `AGENTS.md` — build/test commands, coding conventions, PR expectations.
- `README.md` — purpose, repository layout, setup, local dev flows.
- `DESIGN.md` — core architecture + issue lifecycle + branch tracking.

## Review / Comment structs (pointers)
- Server patch review model: `metis-server/src/domain/patches.rs:56` (`Review`).
- API patch review payload: `metis-common/src/api/v1/patches.rs:61` (`Review`).
- Review draft + inline comments from GitHub:
  - `metis-common/src/models/reviews.rs:7` (`ReviewDraft`).
  - `metis-common/src/models/reviews.rs:24` (`ReviewCommentDraft`).

## patches.rs pointers
- CLI patch commands: `metis/src/command/patches.rs`.
- Server routes: `metis-server/src/routes/patches.rs`.
- Server domain model: `metis-server/src/domain/patches.rs`.
- Server tests: `metis-server/src/test/patches.rs`.
- Background GitHub sync: `metis-server/src/background/poll_github_patches.rs`.
- Shared API types: `metis-common/src/api/v1/patches.rs`.
