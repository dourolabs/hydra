-- Drop the now-unused `default_status_key` column from
-- `metis.projects`. Frontend pre-fill and board-highlight readers were
-- the only behavior anchored on this column; both have been removed
-- alongside the column drop. The `Project` wire type and the row
-- struct in `postgres_v2.rs` no longer reference `default_status_key`,
-- so a Postgres backed by this migration drops the column without
-- leaving any reader behind.
--
-- `IF EXISTS` keeps the migration idempotent on re-runs.
ALTER TABLE metis.projects DROP COLUMN IF EXISTS default_status_key;
