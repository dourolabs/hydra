-- No-op anchor for the `actor_variant_cleanup` Rust migration (see
-- `src/store/migrations/actor_variant_cleanup.rs`). The Rust migration
-- runs in the interleaved migration plan immediately after this SQL
-- version, and `migration_roundtrip` baseline tests anchor their fixtures
-- to a real sqlx migration version — this is that anchor.
SELECT 1;
