-- Add the `suppress_sessions` column to `metis.statuses`. This is the
-- schema-only prerequisite (PR-A of two) for the per-status
-- session-suppression feature: when a status row's `suppress_sessions`
-- is TRUE, the spawn dispatcher (landing in PR-B) skips agent-session
-- creation for issues sitting in that status. `FALSE` (the column
-- default) preserves today's behavior — every existing row is
-- backfilled to FALSE.
--
-- No Rust code reads or writes the column yet; that lands in PR-B. The
-- column-add is decoupled from the consumer so the schema rolls out
-- ahead of the behavior change.

ALTER TABLE metis.statuses ADD COLUMN suppress_sessions BOOLEAN NOT NULL DEFAULT FALSE;
