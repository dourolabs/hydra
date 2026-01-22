# metis-server Guidelines

## Route & Module Layout
- Keep every HTTP handler under `metis-server/src/routes`. Split files by resource (e.g. `jobs.rs`, `repos.rs`) so each module exposes the Axum router plus helper types that stay scoped to that resource.
- Background job logic lives under `metis-server/src/job_engine` and `metis-server/src/background`; keep per-job entrypoints in their own modules so they are easy to wire into schedulers.
- The in-memory store and other shared state live under `metis-server/src/store`—prefer adding helpers there instead of passing raw maps or mutexes through the routes.
- Routes map domain structs in `crate::domain` to the API types in `metis-common::api::v1`; keep those structs in lockstep and update the conversion impls whenever you add fields.
- Application-specific validation (like issue lifecycle checks) belongs in `AppState`; store implementations should only persist and index data without enforcing app-level transitions.

## Logging Policy
- All routes must emit `info!` level logs that let us trace an HTTP request from ingress through response. At a minimum log the handler name, identifiers (e.g. repo, job, user), and the decision taken or status returned.
- Every background job invocation must log at `info!` when it starts (including job name and key parameters) and again when it finishes, capturing whether it succeeded and any high-level outcome so operators can understand what happened in that run.
