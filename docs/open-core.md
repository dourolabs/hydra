# Open-core licensing & feature flags

Hydra is dual-licensed: the core is MIT, and a small set of directories ship
under a proprietary Douro Labs license. This doc names exactly which paths fall
on which side and which cargo features gate the proprietary code.

## License layout

The root `LICENSE` is the authority — it carves out three proprietary trees and
licenses everything else under MIT (Expat):

| Path | License |
|---|---|
| `hydra-server/src/ee/` | Proprietary — see `hydra-server/src/ee/LICENSE` |
| `hydra-build-cache/` | Proprietary — see `hydra-build-cache/LICENSE` |
| `hydra-s3/` | Proprietary — see `hydra-s3/LICENSE` |
| Everything else | MIT |

New code under any of those paths inherits the proprietary license; new code
anywhere else is MIT. Do not move proprietary code outside the carved-out trees.

## What belongs in `ee/` vs. core

`hydra-server/src/ee/` currently contains exactly three things, each behind a
cargo feature:

| Submodule | Purpose | Cargo feature |
|---|---|---|
| `ee/config/kube.rs` | Kubernetes config section | `kubernetes` |
| `ee/job_engine/kubernetes_job_engine.rs` | Kubernetes job engine | `kubernetes` |
| `ee/store/postgres_v2.rs` | Postgres-backed store | `postgres` |

Everything else — including GitHub integration (`routes/github.rs`,
`policy/integrations/github_*.rs`) — stays in **core**. GitHub is not gated.
The rule of thumb: code that ties hydra to a specific cloud-managed dependency
(K8s, Postgres) lives behind a feature in `ee/`; everything else is core.

## Cargo features

Defined in `hydra-server/Cargo.toml`:

| Feature | Enables |
|---|---|
| `postgres` | Postgres store (`ee/store/postgres_v2.rs`) via `sqlx/postgres` |
| `kubernetes` | K8s config + job engine (`ee/config/kube.rs`, `ee/job_engine/kubernetes_job_engine.rs`) via `kube` + `k8s-openapi` |
| `enterprise` | Umbrella that turns on both `postgres` and `kubernetes` |
| `test-utils` | Pulls in `httpmock` + `openssl` for integration test helpers |

Downstream crates must select `features = ["test-utils"]` from `[dev-dependencies]` only, never `[dependencies]` — `test-utils` pulls in `httpmock` + `openssl`, neither of which should ship in a default (non-test) build.

Default builds compile with none of these on. Guard any new `ee/` code with
the matching `#[cfg(feature = "…")]` attribute, and re-check
`cargo check --workspace` (defaults) before submitting — it must keep building
without the feature.

## Postgres migration baselines

See [`docs/rust/migrations.md`](rust/migrations.md) for migration mechanics,
baselines, and the SQLite + Postgres parity rule.
