# Rust Reference Docs

Reference docs for the Rust workspace. Browse what's relevant; this is not a
required-reading list.

## Workspace layout

| Crate | Purpose |
|---|---|
| `hydra` | CLI; one file per subcommand under `hydra/src/command/`. |
| `hydra-server` | Axum HTTP API, background workers, in-memory store by default (Postgres-backed via the `postgres` feature), and `domain` ↔ `hydra-common::api::v1` conversions. Enterprise modules live under `hydra-server/src/ee/`. |
| `hydra-common` | Shared API v1 wire types, domain ids (`HydraId`), and other models reused by server and CLI. |
| `hydra-build-cache` | Build artifact cache used by the CLI / workers. |
| `hydra-bff` | Backend-for-frontend service. |
| `hydra-s3` | S3 storage adapter. |
| `hydra-single-player` | Local single-user variant. |

`hydra-web/` is the TypeScript frontend and is not part of the Cargo workspace
— see `hydra-web/AGENTS.md`.

### Feature flags

`hydra-server` gates external integrations behind features:

- `kubernetes` — `ee/config` and `ee/job_engine` (k8s job engine).
- `postgres` — `ee/store` (Postgres-backed store).
- `enterprise` — umbrella for `postgres + kubernetes`.

Default builds compile without these; check the relevant `#[cfg(feature = ...)]`
blocks before adding new code under `ee/`.

## Topics

- [style.md](style.md) — naming, `HydraId`, libgit2, `///` docs, and where env
  vars are read.
- [idioms.md](idioms.md) — store-owned IDs, mandatory fields over `Option`,
  constructors over builders, secrets via env vars, and `serde(flatten)`.
- [errors-and-logging.md](errors-and-logging.md) — `Result` propagation, when
  panics are acceptable, and the `info!` policy for routes and background jobs.
- [testing.md](testing.md) — the `fmt` / `clippy` / `test` gate, `#[tokio::test]`
  conventions, regression tests, and what *not* to test.
- [cli.md](cli.md) — the global `--output-format` rule and other CLI
  conventions.
