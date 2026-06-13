-- Sister to the Postgres `20260716000000_add_statuses_archived.sql`. Adds
-- the `archived BOOLEAN NOT NULL DEFAULT FALSE` column to `statuses`,
-- the foundation for Phase 3 of the project/status archive feature:
-- `archive_status` flips the column in place (no DELETE on the row, so
-- the FK from `issues_v2.status_sequence` is never tripped) and the
-- cascade flips `issues_v2.deleted` on every non-archived issue at that
-- status. `unarchive_status` flips it back to FALSE.
--
-- Backfilled rows come out FALSE via the column default — no historical
-- status has ever been archived prior to this migration.

ALTER TABLE statuses ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE;
