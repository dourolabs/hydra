-- No-op anchor for the `headless_conversation_backfill` Rust migration
-- (see `src/store/migrations/headless_conversation_backfill.rs`). The
-- Rust migration runs in the interleaved migration plan immediately
-- after this SQL version, and `migration_roundtrip` baseline tests
-- anchor their fixtures to a real sqlx migration version — this is
-- that anchor.

SELECT 1;
