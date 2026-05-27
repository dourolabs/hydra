# Migration testing redesign: versioned baselines + on-store SQL+Rust sequencing

> **Status:** Round 1 design — supersedes `/designs/pre-prod-deploy-test-plan.md` (doc store, round 4 approved 2026-05-26). That design shipped the first iteration of the migration roundtrip harness (`hydra-server/tests/migration_roundtrip.rs`, `migration_baseline.sql`, `seed-migration-fixture`, `.github/workflows/migration-test.yml`). It worked, but four bug-fix iterations on the seed tool during the v0.24.0 release-cut (`i-rhtiyilx`, `i-nytedgut`, `i-xtgwjbdz`) made it clear the single-baseline + dump-and-pin model has structural problems. This doc proposes the next iteration.
>
> **Scope:** design only. Implementation lands as follow-up PRs after approval. The current harness keeps running until the PRs in §9 land.

## §1 Problem statement

The current harness is built around a single baseline file (`migration_baseline.sql`) carrying a `-- baseline-version: <N>` header. The test parses the header, applies sqlx migrations up to `<N>`, executes the fixture body, then rolls migrations forward to HEAD and runs the Rust `migrate-events` pass via a hook. The seed tool (`seed-migration-fixture`) regenerates the fixture at every release.

Four concrete problems with this shape:

1. **Hacky CLI + store extensions exist only because of the single-file model.** The header-parsing trick (`parse_baseline_pin` in `migration_roundtrip.rs`), the seed tool's hard-coded "apply to HEAD then `pg_dump` then prepend the pin" flow, the `--force` flag, and the sha256 `-- migrations-hash:` guard all exist to glue together one fixture file with one pin version. None of them carry intrinsic value — they exist to compensate for the fixture not carrying its schema version in its name.

2. **A single baseline gives us one snapshot per release.** Whenever the release-cutting engineer rolls the fixture forward to the new tag, the prior baseline disappears from the test set. Any migration that bridges multiple prior schema versions (e.g., a backfill that has to cope with rows written at version `V1` and rows written at version `V2 > V1`) is exercised against only the most recent shape. We have no way to incrementally accumulate coverage for older shapes without rebuilding the whole infrastructure.

3. **SQL and Rust migrations live in parallel systems.** `sqlx::migrate!` runs all SQL migrations as one ordered sequence (`postgres_v2::run_migrations`, `SqliteStore::run_migrations`). The one Rust migration we have today (`migration_tool::events::run`) runs separately, spawned as a background `tokio::task` from `build_app_state` (`hydra-server/src/lib.rs:614-621`), *after* SQL has finished. The integration test re-implements the interleaving by calling `MIGRATOR.run_to(pool, pin)`, executing the fixture body, calling `MIGRATOR.run(pool)`, then calling `events::run`. The same logic exists in two places — neither is authoritative, neither composes, and neither allows a sequence like "SQL → Rust → more SQL → more Rust". When the next Rust migration lands that needs to run *between* two SQL versions, both call sites will have to grow that knowledge independently.

4. **Release-process complexity.** The 6-step manual procedure in `hydra-server/AGENTS.md` ("start postgres container → checkout release tag → run seed tool → review fixture diff → commit → stop container") has required four bug-fix iterations on the seed tool during a single release-cut. Most of those bugs were in the seed routine duplicating store-layer logic to produce the desired insert shapes. Every release that introduces a new domain field requires touching the seed tool.

## §2 Goals & non-goals

### Goals

Faithful to the verbatim user-stated design:

1. **Versioned baseline fixture files.** Multiple baselines live in one directory; each is named after a schema version.
2. **Migration test loop.** The test enumerates the directory and, for each baseline in version order, migrates to *just before* the baseline's version, applies the baseline, then continues to *just before* the next one. Final pass rolls to HEAD.
3. **`store/migrations` module.** All in-Rust migration code moves into `hydra-server/src/store/migrations/`. The `migration_tool` module is decommissioned.
4. **Stores own the combined SQL+Rust migration sequence.** Each store's migration entry point interleaves SQL and Rust steps and accepts an **optional version target** so callers can stop early.
5. **Roundtrip driver uses the on-store methods.** The integration test calls the store-level entry point with an `up_to` argument; it does not orchestrate SQL+Rust steps itself.

The combined effect: the same code runs migrations in tests and in production, the test can incrementally add coverage at any historical schema version, and the release-process bash drops to ~zero.

### Non-goals

- **Down / rollback migrations.** sqlx does not support them; we have never relied on them. Out of scope.
- **Performance / load testing of migrations.** Hand-curated baselines are sized for shape coverage, not row volume. A separate framework can scale-test later.
- **Rewriting the existing 77+ SQL migrations.** They keep their filenames, their content, and their `sqlx::migrate!` macro.
- **SQLite parity.** Postgres and SQLite both get the new machinery (see §6) — but if a tradeoff forces a choice, Postgres takes priority. The events Rust migration runs against both today, so both stores need the new entry point or the server's startup hook breaks.
- **CI workflow changes.** `.github/workflows/migration-test.yml` keeps running the same test name (`migration_roundtrip`) against the same Postgres service. The test internals change; the CI trigger does not.

## §3 Versioned baseline directory layout

**Chosen layout:** `hydra-server/tests/fixtures/migration_baselines/<version>__<description>.sql`.

This mirrors sqlx's own filename convention (`20260601000000_review_author_principal.sql`), so the existing developer mental model carries over. `<version>` is the same `u64` timestamp prefix sqlx uses for SQL migrations; `<description>` is a free-form snake_case slug describing what shapes the file exercises.

Each file begins with a one-line header for human readers:

```sql
-- baseline-version: 20260519000000
-- 90 rows. Pre-actor-overhaul shapes: bare-string assignees on issues_v2,
-- prefixed `agents/swe` assignees, conversation_events_v2 user/assistant
-- rows that the events Rust migration will move into session_events_v2.
INSERT INTO metis.issues_v2 (...) VALUES (...);
...
```

The test still derives the version from the **filename**, not the header — the header is documentation, not a parse target. (The current `parse_baseline_pin` helper goes away.)

**Schema-state contract:** each baseline contains INSERTs valid against the schema *just after migration `<version>` has applied* — i.e., the file's row shapes match the schema state at version `<version>`. The test loop applies migrations up to and including the file's version, then executes the file. (See §4 for the exact algorithm.)

Illustrative directory state after the first follow-up baseline ships:

```
hydra-server/tests/fixtures/migration_baselines/
    20260519000000__pre_actor_overhaul.sql        # current single fixture, renamed
    20260601000000__post_review_author_principal.sql  # hypothetical, illustrative
```

The lone existing fixture `migration_baseline.sql` becomes `20260519000000__pre_actor_overhaul.sql` (file rename + header tweak) — see §8 item 4.

## §4 Test loop algorithm

```rust
let mut baselines: Vec<Baseline> =
    list_files("hydra-server/tests/fixtures/migration_baselines/")
        .map(parse_baseline_filename)
        .collect();
baselines.sort_by_key(|b| b.version);

reset_database(&pool).await?;

let mut prev_version: Option<u64> = None;
for b in &baselines {
    // Validate: filenames must be strictly increasing and every version must
    // correspond to a real SQL migration on this checkout (catches "future"
    // baselines left behind by a merged-then-reverted branch).
    if let Some(p) = prev_version { assert!(b.version > p, "baselines out of order"); }
    assert!(MIGRATOR.iter().any(|m| m.version == b.version),
        "baseline {} has no matching sqlx migration on this checkout", b.version);

    // Migrate up to and including this baseline's version. The fixture INSERTs
    // are valid against the schema state at this version.
    store.run_migrations(Some(b.version)).await?;
    sqlx::raw_sql(&b.body).execute(&pool).await?;

    prev_version = Some(b.version);
}

// Final pass: roll to HEAD, including any Rust migrations whose version is
// beyond the last baseline.
store.run_migrations(None).await?;

// §3.3 store-level smoke — preserved verbatim from the current design.
assert_store_level_smoke(&pool).await?;
```

**Why "up to and including this baseline's version", not "just before"?** The verbatim design says "migrate up to *just before* that version, apply the baseline, then continue". Both shapes work, but "up to and including" is simpler:

- It means each baseline lives at a real sqlx migration version we already know exists.
- "Just before" requires deriving the *predecessor* version from `MIGRATOR.iter()` (sqlx's `run_to` targets are specific migration versions; there is no "stop just shy of version N" mode), which adds a helper and an off-by-one risk.
- Either way, the test exercises every migration with version `> b.version` against the data in the file, which is what we want.

**Equivalent in plain English:** before each baseline file, the database is at the schema state where that file's INSERTs are valid; *every migration with a higher version* sees the file's rows and has to handle them correctly.

**Edge cases:**

- *Baseline at a SQL-migration version that performs a backfill.* The baseline contains the rows in their pre-backfill source shape; we apply migrations up to and including the backfill migration; the backfill migration itself never sees those rows (they were inserted after it ran). To exercise a backfill, the baseline must be one version *before* the backfill — the file is named after the version where the source shape is valid.
- *Baseline whose version sits between two SQL migrations with no Rust step in that range.* Trivial — `store.run_migrations` is just SQL.
- *Empty `migration_baselines/` directory.* The loop runs zero iterations; the final `run_migrations(None)` still hits HEAD. The test reduces to a smoke that "all migrations apply cleanly on an empty DB" — fine to keep as a sanity check.
- *Baseline at a version newer than the latest migration on this checkout.* The validation assertion above fires; the test errors clearly rather than silently skipping.
- *Two baselines at the same version.* Validation rejects (`baselines out of order` covers strict-increasing).

## §5 The `hydra-server/src/store/migrations/` module

A new module replaces `hydra-server/src/migration_tool/`. Layout:

```
hydra-server/src/store/migrations/
    mod.rs        // module surface: trait, registry, helpers
    events.rs     // lifted from migration_tool/events.rs, signature simplified
```

Each Rust migration is a struct implementing a small trait:

```rust
pub trait RustMigration: Send + Sync {
    /// The sqlx migration version this Rust step must run *after*. The
    /// migration framework will run this step the moment SQL migrations
    /// reach this version, before any higher-versioned SQL migration runs.
    fn version(&self) -> u64;

    /// Short identifier for logging.
    fn name(&self) -> &'static str;

    /// Apply the migration. Must be idempotent — re-running on
    /// already-migrated data is required to be a no-op (matches the
    /// existing `events::run` per-session-skip contract).
    async fn run(&self, backend: &Backend) -> anyhow::Result<()>;
}
```

The module exposes a static-ordered registry:

```rust
pub fn rust_migrations() -> &'static [&'static dyn RustMigration] {
    static EVENTS: events::EventsMigration = events::EventsMigration;
    &[&EVENTS]
}
```

Today this registry has one entry; over time it gains one entry per checked-in Rust migration. Order is by `version()`; the registry is sorted at compile time (single source of truth: the `&[]` literal).

The `Backend` enum survives the lift (`enum Backend { Sqlite(SqlitePool), Postgres(PgPool) }`), since the `RustMigration` trait method needs to dispatch on backend. It moves from `migration_tool/mod.rs` to `store/migrations/mod.rs`.

**Signature simplification (behavioral change worth flagging).** The current `events::run` signature is `pub async fn run(backend: &Backend, dry_run: bool, up_to: Option<DateTime<Utc>>) -> Result<Vec<EventPlanEntry>>`. The `dry_run` and `up_to` knobs were only used by the deleted `hydra-migrate-sessions` CLI binary (patch `p-fnecthet`, 2026-05-27). No live caller passes anything but `false` and `None`. The new trait method drops both:

```rust
async fn run(&self, backend: &Backend) -> anyhow::Result<()>;
```

The implementation underneath keeps the partitioning + per-session-skip logic exactly as it is — just no longer parameterized by `dry_run` / `up_to`. The `EventPlanEntry` return type also goes away (no caller reads it). This is a small but real behavioral simplification: if some future ops task wants a dry-run view of which rows would move, they'd need to bring the knob back — but doing so will be cleaner once the trait shape is in place.

## §6 Store-owned combined migration methods

**Chosen: free function per store impl** (Option B in the issue brief), not a trait method.

The shape:

```rust
// hydra-server/src/ee/store/postgres_v2.rs
pub async fn run_migrations(pool: &PgStorePool, up_to: Option<u64>) -> Result<()>;

// hydra-server/src/store/sqlite_store.rs
pub async fn run_migrations(pool: &SqlitePool, up_to: Option<u64>) -> Result<()>;
```

Today both already have `run_migrations` free functions — both grow the `up_to` parameter and the interleaving logic.

**Why free function, not trait method:**

- The `Store` trait is large and load-bearing; adding a method ripples through every impl (Memory, SQLite, Postgres, the test doubles in `tests/`). The Memory store has no migrations.
- Callers know the concrete pool type at the call site (`AppState`-construction in `lib.rs` dispatches on `StorageConfig`; the integration test instantiates `PostgresStoreV2` directly). Polymorphism over `Store` would buy nothing.
- A free function is the path of least resistance for the cross-cutting "interleave a static registry of Rust migrations with sqlx's `Migrator`" logic — the body can be implemented once as a private helper and the two free functions become two-liners.

**Algorithm (the shared helper):**

```rust
async fn interleave<Run, RunRust>(
    sql_versions: impl Iterator<Item = u64>,    // sqlx Migrator version sequence
    rusts: &[&dyn RustMigration],
    up_to: Option<u64>,
    backend: Backend,
    mut run_sql_to: Run,                         // calls MIGRATOR.run_to(pool, v)
    mut run_rust: RunRust,
) -> Result<()>
where
    Run: FnMut(u64) -> BoxFuture<'_, Result<()>>,
    RunRust: FnMut(&dyn RustMigration) -> BoxFuture<'_, Result<()>>,
{
    let target = up_to.unwrap_or(u64::MAX);
    let mut next_rust = 0usize;

    for v in sql_versions.take_while(|&v| v <= target) {
        run_sql_to(v).await?;
        while next_rust < rusts.len() && rusts[next_rust].version() <= v {
            run_rust(rusts[next_rust]).await?;
            next_rust += 1;
        }
    }

    // Drain any Rust migrations whose version <= target but didn't have a
    // matching SQL version to attach to (e.g., a Rust migration whose
    // `version` exceeds the highest SQL version).
    while next_rust < rusts.len() && rusts[next_rust].version() <= target {
        run_rust(rusts[next_rust]).await?;
        next_rust += 1;
    }
    Ok(())
}
```

The Postgres free function wires `MIGRATOR.run_to(pool, v)` and `rust_migration.run(&Backend::Postgres(pool.clone()))`; the SQLite free function does the same with its `MIGRATOR` and `Backend::Sqlite`. Both backends get the same machinery (see §10 on SQLite parity).

**Semantics:**

- `up_to: None` → apply every SQL migration and every Rust migration. Equivalent to today's `run_migrations` plus the startup `spawn_startup_events_migration`, sequenced inline rather than via `tokio::spawn`.
- `up_to: Some(v)` → apply every SQL migration with version `≤ v` and every Rust migration with `version() ≤ v`. If `v` does not match a real SQL migration version, the loop simply runs to the highest version `≤ v` (sqlx's `run_to` requires an exact version, so the loop calls it with each version individually rather than aiming at `v` directly).
- Idempotency: the Rust migrations are required to be no-op idempotent (the trait contract in §5). The SQL migrations are idempotent by sqlx's tracking table. So `run_migrations(None)` can be called on a partially-migrated DB and finishes the job.

**Side note on a guardrail [[migrations]]:** SQLite migrations that reorder columns must not `INSERT INTO new_table SELECT * FROM old_table` — column order in `SELECT *` is unstable across schema changes and silently produces corruption. Out of scope for this design (we're not writing SQL migrations here), but the new `store/migrations/` module's docstrings will reference the guardrail since the migration audience will be reading them.

## §7 Roundtrip driver rewrite

`hydra-server/tests/migration_roundtrip.rs` shrinks substantially. The new shape:

```rust
#![cfg(feature = "postgres")]

#[tokio::test]
#[ignore]
async fn migration_roundtrip() -> Result<()> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping.");
        return Ok(());
    };
    let pool = PgPool::connect(&database_url).await?;
    reset_database(&pool).await?;

    let baselines = load_baselines("hydra-server/tests/fixtures/migration_baselines")?;
    for b in &baselines {
        postgres_v2::run_migrations(&pool, Some(b.version)).await?;
        sqlx::raw_sql(&b.body).execute(&pool).await?;
    }
    postgres_v2::run_migrations(&pool, None).await?;

    assert_schema_invariants(&pool).await?;
    assert_data_shape_invariants(&pool).await?;
    assert_store_level_smoke(&pool).await?;   // preserved verbatim — round-2
                                              //   acceptance from the prior design
    Ok(())
}
```

What's gone:

- `parse_baseline_pin` — version is in the filename.
- `run_external_migrations` — `run_migrations(None)` runs Rust migrations as part of its sequence.
- The `-- migrations-hash:` sha256 check on the fixture — each baseline is immutable; if a SQL migration with a lower version is added or modified later, *that change* is what we want the test to surface.
- The single-file `migration_baseline.sql` — replaced by the versioned directory (see §8 item 4).

What's preserved:

- Schema invariants (§3.1 of the prior design) and data-shape invariants (§3.2).
- The §3.3 store-level smoke: read the fixture rows back through `Store`/`ReadOnlyStore` APIs, verify typed deserialization (`Principal`, `SessionMode`, kebab-case rel types). See memory [[migration-test-read-migrated]] — this remains the round-2 acceptance criterion and must not regress.
- `#[ignore]` + `#![cfg(feature = "postgres")]` gating + `DATABASE_URL` discovery — unchanged.

The new test is small enough (~150 lines vs. ~900 today) that the assertions can largely be inlined; helpers stay only for the smoke (§3.3), which is the bulk of the surviving complexity.

## §8 Cleanup audit (file-by-file)

| # | Item | Why unnecessary | What to remove | Subtle deps to watch |
|---|------|-----------------|----------------|----------------------|
| 1 | `hydra-server/src/migration_tool/mod.rs` and `events.rs` | All in-Rust migration code moves to `store/migrations/`. `Backend` enum moves with it. | Delete both files; delete `pub mod migration_tool;` from `hydra-server/src/lib.rs`. | (a) `lib.rs:78,90` references `migration_tool::Backend` in `spawn_startup_events_migration` — update to `store::migrations::Backend`, then delete the spawn (item 6). (b) The current `migration_roundtrip.rs` imports `hydra_server::migration_tool::{Backend, events}` — gone after item 5. (c) Any `pub use` re-exports in `lib.rs` — grep before deleting. |
| 2 | `hydra-server/src/bin/seed-migration-fixture/{main.rs,seed.rs}` plus the `[[bin]]` entry in `hydra-server/Cargo.toml:21-24` | Versioned baselines are written **once per release** by hand-curating (or scripting a one-shot generation, see below) — there is no "roll forward at every release" cadence. The seed-then-`pg_dump` flow exists only because the single-baseline model requires a fresh dump each release. | **Delete.** Drop the binary, the `[[bin]]` entry, and the `src/test_seed/` module if any landed alongside (search the tree). | Re-rolling old baselines after delete is impossible without restoring the tool — that's the point. Old baselines are immutable artifacts; if we want a *new* baseline at a future version, the engineer hand-writes the INSERTs against that schema (or, one-shot, writes a small script and discards it). **Position:** delete. Do not repurpose. Keeping the tool around as "for ad-hoc use" invites the same 4-iteration churn we just paid; the value of versioned baselines is that they're immutable. |
| 3 | `hydra-server/AGENTS.md` §"Migration baseline" (currently lines 21-37) | The 6-step manual release procedure is the operational tax that motivated this redesign. With versioned baselines, the release-cut workflow gets no new baseline; baselines are added when a *migration author* decides a new shape needs coverage. | Delete the section. Replace with a one-paragraph note explaining that baselines live in `migration_baselines/`, are immutable once committed, and are added by migration authors at PR time when a new shape requires coverage. | The `hydra-server/AGENTS.md` heading currently links to `/designs/pre-prod-deploy-test-plan.md` — update to point at this doc (`docs/migration-testing-redesign.md`). |
| 4 | `hydra-server/tests/fixtures/migration_baseline.sql` | Replaced by the versioned-baselines directory. Its row content is still valuable — it's the only baseline we have. | Rename to `hydra-server/tests/fixtures/migration_baselines/20260519000000__pre_actor_overhaul.sql`. Drop the `-- migrations-hash:` header line. Keep the rest of the file unchanged (the INSERTs are already valid against the schema state at version `20260519000000`). | The current header says `-- baseline-version: 20260519000000`; the new naming makes the comment redundant but harmless — keep it for readability or remove, either fine. |
| 5 | `hydra-server/tests/migration_roundtrip.rs` (current 900-line shape) | Driver moves to the §7 shape. `parse_baseline_pin`, `run_external_migrations`, the migrations-hash assertion, the `MIGRATOR.run_to` + `MIGRATOR.run` pair — all gone. | Replace the file body. Keep the `#![cfg(feature = "postgres")]` gate, the `#[ignore]` attr, the `DATABASE_URL` discovery, and the §3.3 store-level smoke helpers. | The file `include_str!`s the fixture path at line 45 — that line goes away; baselines are now read at runtime by directory enumeration so the test can pick up new files without recompilation. |
| 6 | `spawn_startup_events_migration` in `hydra-server/src/lib.rs:614-621` and its two call sites at `:78` and `:90` | `run_migrations(None)` at startup runs the Rust migration inline. No more `tokio::spawn` background-task fire-and-forget. | Delete the function. Replace the two call sites with `…::run_migrations(&pool, None).await?` (already what the code is doing for SQL — just becomes the combined call). | **Behavioral change:** today the Rust migration runs as a *background* task, so a failure logs `warn!` but does not block startup. The new behavior blocks startup until the Rust migration completes. For events specifically this is OK — the pass is idempotent and fast (skips already-migrated sessions). But flag this explicitly in the PR description; an operator's expectation that "server boots even if the events backfill is mid-pass" no longer holds. If a future Rust migration is *long* (minutes), we'd need to bring back a spawn-and-forget mode — but that's a future migration's problem, not this design's. |
| 7 | `hydra-migrate-sessions` leftover references | The binary was deleted 2026-05-27 (patch `p-fnecthet`). | Grep the tree for `hydra-migrate-sessions` and prune lingering hits: `prompts/`, `scripts/`, deploy docs, any `/playbooks/*.md` step still referencing it. | The string also appears in code comments (e.g., `migration_tool/mod.rs:12`). Comments in deleted files go away with item 1; check the rest. |
| 8 | `.github/workflows/migration-test.yml` | Unchanged — still runs `migration_roundtrip` against a `postgres:16` service. | No change. | Call out explicitly in the rollout PR description so a reviewer doesn't go looking for a CI delta. |
| 9 | `/designs/pre-prod-deploy-test-plan.md` (doc store) | Superseded by this doc. | Update the prior doc's header: add a `> **Status: superseded by `docs/migration-testing-redesign.md`** as of <date>` banner. Do not delete; the round-1..round-4 history is referenceable. | Done by the PM at design-approval time; the SWE PRs need not touch the doc store. |

Items the SWE should double-check during PR-A and add to the audit if found:

- `Cargo.toml` `[[bin]]` entry for `hydra-migrate-sessions` (already removed per `p-fnecthet`, but `git grep` to confirm).
- Any imports of `hydra_server::migration_tool` from outside `hydra-server` (e.g., from `hydra-bff` or `hydra/`). None should exist; if they do, re-point to `store::migrations::`.

## §9 Migration / rollout strategy

A 4-PR chain. Each PR is independently mergeable and CI-green; the harness keeps running throughout.

- **PR-A: Lift `migration_tool/` into `store/migrations/` with no behavioral change.** Move `mod.rs` and `events.rs` under the new path; the `events::run` function keeps its current signature for this PR (the simplification in §5 lands in PR-B). Add `pub use crate::store::migrations as migration_tool;` in `hydra-server/src/lib.rs` so external callers (and the integration test) keep compiling unchanged. Production startup keeps calling `spawn_startup_events_migration` against the new module path. No new tests; existing tests must pass.

- **PR-B: Add `run_migrations(pool, up_to: Option<u64>)` to each store impl.** Introduce the `RustMigration` trait and the registry; refactor `events.rs` into a struct that implements the trait (signature simplification from §5 lands here). Both Postgres and SQLite `run_migrations` grow the `up_to` parameter and interleave SQL+Rust. Production startup (`lib.rs:78,90`) switches to `run_migrations(&pool, None)` and deletes `spawn_startup_events_migration`. Integration test is *not yet* rewritten — it keeps calling `MIGRATOR.run_to`/`MIGRATOR.run` to maintain test-side stability through this PR. (The Rust-migration call from the test moves to using the new trait via `run_migrations(None)`, dropping `run_external_migrations`.)

- **PR-C: Restructure fixtures into `migration_baselines/`. Rewrite the test loop.** Rename `migration_baseline.sql` to `migration_baselines/20260519000000__pre_actor_overhaul.sql`. Rewrite `migration_roundtrip.rs` to the §7 shape. Delete `parse_baseline_pin` and the migrations-hash assertion. After this PR, the test is the small new shape; the §3.3 smoke is preserved verbatim.

- **PR-D: Delete the regen tool, the `migration_tool` re-export, and the AGENTS.md release section.** Drop `src/bin/seed-migration-fixture/` and the `[[bin]]` entry. Remove the `pub use … as migration_tool` backcompat shim from PR-A. Rewrite `hydra-server/AGENTS.md` §"Migration baseline". Update any other docs / playbook references found via grep.

After PR-D lands, the new design is fully in place. PRs are nominally sequential (each depends on the previous's API), but PR-D's pieces could be split further if review wants smaller diffs (e.g., delete the regen tool first, then the AGENTS.md edit, then the backcompat shim — three small PRs).

This split is illustrative; the implementer may merge or split further to match review preferences. The doc just confirms the work is incremental and reversible at each step.

## §10 Risks / open questions

- **Startup blocking on the Rust migration.** §8 item 6. The events pass is idempotent and short on real data, so blocking startup is acceptable today. If a future Rust migration is long-running, we'll need a "background after startup" mode again. **Decision:** accept the change for now. Flag in PR-B's description.

- **Baseline rot from a re-ordered SQL migration.** If someone adds a new SQL migration whose version is *less than* an existing baseline's version (e.g., backfilling a migration with timestamp `20260518000000` after `20260519000000__pre_actor_overhaul.sql` exists), the older baseline's INSERTs may no longer be valid against the (now-extended) schema-at-its-version. This is the standard sqlx warning ("don't reorder migrations") elevated to a test-failure: the test will fail loudly, which is exactly the right behavior — the engineer adding the out-of-order migration needs to know they're invalidating a baseline. Validation in §4 catches "future" baselines; loud failure catches "past" reorders. Acceptable.

- **SQLite parity.** **Decision: yes, SQLite gets the same machinery.** The events Rust migration runs against SQLite today (via `Backend::Sqlite`), so the SQLite startup path also needs `run_migrations(pool, None)` to keep that behavior. SQLite does not get a parallel test harness in this design — Postgres-only roundtrip continues. If we find SQLite-specific migration bugs later, a parallel test is a small follow-up. SQLite's `run_migrations` free function gains `up_to` symmetrically so the API surface is uniform.

- **Cross-baseline coverage matrix size.** How many baselines do we keep? **Position:** add baselines *case-by-case*, not one per release. The rule on PR review: "if your migration introduces a new source shape that doesn't appear in any existing baseline, add a baseline that does." That keeps the directory small and the test fast. Initial state is one baseline (the renamed `pre_actor_overhaul.sql`); growth is migration-author-driven. If the directory ever exceeds, say, ~10 baselines and the test gets slow, we can revisit by retiring the oldest baselines (deletion is allowed — they're not load-bearing once their source shapes no longer appear in migration logic).

- **One-shot generation of new baselines, given the seed tool is deleted.** If a future baseline genuinely needs hundreds of rows shaped by complex domain logic, hand-writing INSERTs is painful. The fallback is: write a one-off Rust script under `scripts/` that uses `PostgresStoreV2` to seed and `pg_dump`s the result, run it once, commit the output, delete the script. This is the "scripts are disposable" pattern — the persistent tool was the bug, not the seeding approach.

- **The new `RustMigration` trait + registry's compile-time order.** A new Rust migration is added by adding a struct + an entry in the `&[...]` literal in `rust_migrations()`. The order in the literal must match `version()` ascending. If a developer forgets to sort the literal, the interleave loop's `take_while`/`while` invariant breaks. **Mitigation:** add a `debug_assert!` in `rust_migrations()` (or in the helper) that the slice is sorted by `version()`. Tiny and catches the bug at the first test that calls `run_migrations`.

- **What if a Rust migration must run *before* its same-version SQL migration?** The `version()` contract in §5 is "the SQL version this Rust migration must run *after*". That implies SQL runs first at any shared version. We have no current case where a Rust migration needs to run *before* a same-version SQL step; if one ever appears, the trait would need a `RunBefore { sql_version: u64 } | RunAfter { sql_version: u64 }` discriminator. Note as a potential future extension; not in scope here.

---

**End of design.**
