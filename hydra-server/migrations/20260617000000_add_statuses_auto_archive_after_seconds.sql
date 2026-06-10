-- Add the `auto_archive_after_seconds` column to `metis.statuses`. This is
-- the plumbing PR (PR-1 of three) for per-status auto-archive: when set to
-- a positive integer, a periodic worker (landing in a follow-up PR) will
-- archive issues that have sat in this status for at least that many
-- seconds. `NULL` (the default) leaves the feature off.
--
-- No backfill: every existing status row starts with the feature off.
-- Behavior is gated on the worker that ships in PR-2.

ALTER TABLE metis.statuses ADD COLUMN auto_archive_after_seconds BIGINT;
