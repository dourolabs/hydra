# hydra-server Guidelines

## Route & Module Layout
- Keep every HTTP handler under `hydra-server/src/routes`. Split files by resource (e.g. `jobs.rs`, `repos.rs`) so each module exposes the Axum router plus helper types that stay scoped to that resource.
- Background job logic lives under `hydra-server/src/job_engine` and `hydra-server/src/background`; keep per-job entrypoints in their own modules so they are easy to wire into schedulers.
- Assignment agents require a dedicated queue: declare the queue under `background.agent_queues` and set `background.assignment_agent` to its name so unassigned issues always share a stable routing target.
- The in-memory store and other shared state live under `hydra-server/src/store`—prefer adding helpers there instead of passing raw maps or mutexes through the routes.
- Routes map domain structs in `crate::domain` to the API types in `hydra-common::api::v1`; keep those structs in lockstep and update the conversion impls whenever you add fields.
- Application-specific validation (like issue lifecycle checks) belongs in `AppState`; store implementations should only persist and index data without enforcing app-level transitions.

## Integration Testing Guidelines
- Integration tests must use `worker_run` and the hydra CLI to perform actions, simulating real agent behavior.
- Agent status transitions (e.g., setting an issue to Failed) should happen via the CLI inside a worker, not via direct API calls.
- When testing failure/rejection cascades, include dependent issues (blocked-on, children) to verify cascade behavior.
- Tests should be end-to-end simulations of real workflows, not shortcuts using internal APIs.

## Logging Policy
- All routes must emit `info!` level logs that let us trace an HTTP request from ingress through response. At a minimum log the handler name, identifiers (e.g. repo, job, user), and the decision taken or status returned.
- Every background job invocation must log at `info!` when it starts (including job name and key parameters) and again when it finishes, capturing whether it succeeded and any high-level outcome so operators can understand what happened in that run.

## Migration baseline
The `seed-migration-fixture` binary regenerates `hydra-server/tests/fixtures/migration_baseline.sql`, the populated fixture that PR-1's `migration_roundtrip` test reads. It applies every migration on the current checkout to a fresh Postgres, runs the deterministic seed in `src/test_seed/mod.rs`, then writes a single file whose header records the pin (`-- baseline-version: <N>`) and a sha256 of the migrations tree (`-- migrations-hash: <hex>`). The body is `pg_dump --data-only --inserts --column-inserts --schema=metis` of the seeded DB. See `/designs/pre-prod-deploy-test-plan.md` §5 for the long-form description.

Run this once per release-cut, from a fresh checkout of the release tag, against a dedicated empty Postgres:

```
docker run -d --name pg-seed -e POSTGRES_PASSWORD=test -p 5432:5432 postgres:16
DATABASE_URL=postgres://postgres:test@localhost:5432/postgres \
    cargo run -p hydra-server --features postgres --bin seed-migration-fixture -- \
        --database-url $DATABASE_URL --force
docker rm -f pg-seed
git add hydra-server/tests/fixtures/migration_baseline.sql
git diff --cached  # human-review the fixture diff
git commit -m "Roll migration baseline to vX.Y.Z"
```

The tool refuses to overwrite an existing fixture whose `-- migrations-hash:` does not match the current tree, or to drop a populated `metis` schema — pass `--force` only when you are deliberately re-running from the right checkout.
