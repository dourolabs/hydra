-- Add the `archived` column to `metis.statuses`. Foundation for Phase 3
-- of the project/status archive feature: `archive_status` flips the
-- column in place (the row stays in the table so the FK from
-- `metis.issues_v2.status_sequence` is never tripped) and cascade-flips
-- `metis.issues_v2.deleted` on every non-archived issue at that status.
-- `unarchive_status` flips it back to FALSE.
--
-- Backfilled rows come out FALSE via the column default — no historical
-- status has ever been archived prior to this migration.

ALTER TABLE metis.statuses ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE;
