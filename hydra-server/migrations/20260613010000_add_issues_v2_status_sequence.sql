-- Add `metis.issues_v2.status_sequence` and backfill it by joining the
-- existing `issues_v2.status` (a `StatusKey` text) against
-- `metis.statuses(project_id, key)` to recover the per-project
-- `sequence` id added in the sibling `20260613000000_create_statuses`
-- migration. See [[i-bqimglba]] for the rationale: storing issues
-- against the `sequence` id rather than the text `key` is what makes
-- a future `StatusKey` rename safe.
--
-- PR 1 of 4: no application code reads or writes `status_sequence`
-- yet. The column is left nullable and unconstrained (no FK to
-- `metis.statuses`) so the rest of the chain can roll back without
-- restoring data. PR 3 tightens both: it drops `issues_v2.status`,
-- adds the FK, and sets `status_sequence` to NOT NULL once the cutover
-- is complete.
--
-- Pre-flight NULL guard at the tail: every legacy issue row must have
-- a matching status row resolvable via `(project_id, status)`. If any
-- row remains NULL after the backfill, abort the migration loudly
-- rather than silently leaving issues orphan-pointing into nothing.

ALTER TABLE metis.issues_v2 ADD COLUMN IF NOT EXISTS status_sequence BIGINT NULL;

-- The `status_sequence IS NULL` guard makes the UPDATE idempotent if
-- the body is ever re-applied: already-backfilled rows are untouched,
-- so the body can re-run as a no-op.
UPDATE metis.issues_v2
   SET status_sequence = s.sequence
  FROM metis.statuses s
 WHERE s.project_id            = metis.issues_v2.project_id
   AND s.key                   = metis.issues_v2.status
   AND metis.issues_v2.status_sequence IS NULL;

CREATE INDEX IF NOT EXISTS issues_v2_project_status_sequence_idx
    ON metis.issues_v2 (project_id, status_sequence);

DO $$
DECLARE
    null_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO null_count
      FROM metis.issues_v2
     WHERE status_sequence IS NULL;
    IF null_count > 0 THEN
        RAISE EXCEPTION
            'add_issues_v2_status_sequence: refusing to complete; % NULL status_sequence row(s) remain after backfill. Inspect orphan (project_id, status) pairs in metis.issues_v2 that have no matching metis.statuses row.',
            null_count;
    END IF;
END $$;
