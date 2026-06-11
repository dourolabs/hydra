-- Add the `max_simultaneous_sessions` column to `metis.statuses`. Caps
-- the number of concurrently-active sessions (interactive + headless,
-- across all agents) whose issue currently sits in this status. NULL
-- (the default) leaves the cap off; a positive integer caps active
-- sessions to at most that count.
--
-- No backfill: every existing status row starts with no cap. Existing
-- sessions above a freshly-lowered cap continue to run; only new spawns
-- are blocked until the active count drops below the cap.

ALTER TABLE metis.statuses ADD COLUMN max_simultaneous_sessions BIGINT;
