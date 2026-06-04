# Migrations

`hydra-server` ships two store backends and both must evolve together. This
doc names where migrations live, how the roundtrip tests work, and the rule
every new migration is held to.

## Both stores are mandatory

`hydra-server` ships two `Store` impls:

- **SQLite** — `hydra-server/src/store/sqlite_store.rs` (default).
- **Postgres** — `hydra-server/src/ee/store/postgres_v2.rs` (gated by the
  `postgres` cargo feature; see [open-core.md](../open-core.md)).

Every feature that touches persistence MUST land migrations and `Store` impl
changes for BOTH backends in the same PR. A Postgres-only or SQLite-only
change is an incomplete feature.

## Where migrations live

- Postgres SQL: `hydra-server/migrations/<version>__<description>.sql` —
  driven by the sqlx `Migrator` in
  [`ee/store/postgres_v2.rs`](../../hydra-server/src/ee/store/postgres_v2.rs).
- SQLite SQL: `hydra-server/sqlite-migrations/<version>__<description>.sql` —
  driven by the sqlx `Migrator` in
  [`store/sqlite_store.rs`](../../hydra-server/src/store/sqlite_store.rs).
- Rust-code migrations interleave with SQL at a declared version via the
  planner in
  [`store/migrations/mod.rs`](../../hydra-server/src/store/migrations/mod.rs)
  (existing impls: `events.rs`, `actor_variant_cleanup.rs`). Each must be
  idempotent — the server's boot path re-runs the full registry.

Apply paths, callable for tests and used on server boot:

- Postgres: `hydra_server::store::postgres_v2::run_migrations(&pool, up_to)`.
- SQLite: `hydra_server::store::sqlite_store::run_migrations(&pool, up_to)`.

`up_to == None` rolls to HEAD; `Some(version)` stops at a sqlx version.

## Migration tests

Two integration tests interleave versioned baseline fixtures with migration
runs to simulate real upgrade paths over real data:

- Postgres:
  [`tests/migration_roundtrip.rs`](../../hydra-server/tests/migration_roundtrip.rs)
  — gated behind `#[cfg(feature = "postgres")]` AND `#[ignore]`, requires
  `DATABASE_URL` pointing at a Postgres instance, opt-in via
  `cargo test --features postgres -- --ignored`.
- SQLite:
  [`tests/migration_roundtrip_sqlite.rs`](../../hydra-server/tests/migration_roundtrip_sqlite.rs)
  — runs under the default `cargo test --workspace` against
  `sqlite::memory:` (no `#[ignore]`, no feature gate).

Baseline fixtures live alongside each test:
`hydra-server/tests/fixtures/migration_baselines/` (Postgres) and
`migration_baselines_sqlite/` (SQLite), named
`<version>__<description>.sql` after the highest sqlx migration version
against which their `INSERT` shapes are valid. The test walks baselines in
order, applies migrations up to each pin, runs the baseline INSERTs, then
rolls to HEAD.

Each test asserts (see the top-of-file comments for the precise algorithm):

1. **Schema invariants** — columns / tables added / dropped / tightened.
2. **Data-shape invariants** — SQL-level read-back of backfilled rows.
3. **Store / domain smoke** — high-level `Store` API reads confirm typed
   domain values deserialize, plus a fresh CREATE → read-back exercises the
   post-migration write paths.

## Every new migration MUST have a migration test

Non-optional. For each new migration: either extend an existing baseline
with the row shapes the new migration cares about, or add a new baseline at
the new migration's version; then add an assertion in the matching
`migration_roundtrip*.rs` confirming the schema / data-shape change. The
roundtrip tests are the only thing that runs each migration against
realistic prior-version data — without coverage, a regression in a
rarely-touched migration ships silently the next time someone refactors the
store.
