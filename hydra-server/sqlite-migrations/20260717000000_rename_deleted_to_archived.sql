-- Rename the per-entity soft-delete column `deleted` to `archived` on every
-- entity except `projects` and `statuses`. The sibling PR ([[i-tozlzhui]]) owns
-- the project + status side of the rename, including the cascade-archive
-- semantics; this migration is the mechanical hygiene rename for the ten
-- entities that just carry a flag.
--
-- SQLite supports `ALTER TABLE ... RENAME COLUMN` (since 3.25.0); the rename
-- is in-place and preserves indexes and partial unique indexes that reference
-- the column. No data movement, so the [[migrations]] guardrail against
-- `INSERT INTO new_table SELECT *` does not apply here.
--
-- Idempotency: SQLite rejects a second `RENAME COLUMN` because the source
-- column is gone after the first run. The `IF EXISTS`-style guard is not
-- available, so re-running this migration body errors. That is fine for the
-- sqlx migrator: it checkpoints `_sqlx_migrations` after a successful apply
-- and never re-runs an applied SQL migration. The Rust migration interleave
-- only re-invokes `RustMigration::run`, never SQL bodies.

ALTER TABLE repositories_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE users_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE issues_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE patches_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE tasks_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE documents_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE agents RENAME COLUMN deleted TO archived;
ALTER TABLE labels RENAME COLUMN deleted TO archived;
ALTER TABLE conversations RENAME COLUMN deleted TO archived;
ALTER TABLE triggers RENAME COLUMN deleted TO archived;
