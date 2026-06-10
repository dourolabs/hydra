-- Backfill `assignee = NULL` (both the legacy `assignee` TEXT column
-- and the typed `assignee_principal` JSONB column) on every latest
-- `metis.issues_v2` row that belongs to the default project and is
-- currently in one of the three terminal statuses (`closed`,
-- `dropped`, `failed`). Terminal default-project statuses carry
-- `on_enter.clear_assignee = true`, so `apply_status_on_enter` nulls
-- the assignee on every new transition; this migration is the one-
-- time backfill for rows that landed in a terminal status before that
-- automation existed, so live data matches the post-cutover invariant.
--
-- Scoped to `is_latest = TRUE` so historic versions retain whatever
-- assignee they were created with — the invariant is about
-- application-visible state, not the audit log.
--
-- Idempotent: the `assignee IS NOT NULL OR assignee_principal IS NOT NULL`
-- guard makes a re-run a no-op once every targeted row has both
-- columns nulled.

UPDATE metis.issues_v2
SET assignee = NULL,
    assignee_principal = NULL
WHERE is_latest = TRUE
  AND project_id = 'j-defaul'
  AND status_sequence IN (
      SELECT sequence
      FROM metis.statuses
      WHERE project_id = 'j-defaul'
        AND key IN ('closed', 'dropped', 'failed')
  )
  AND (assignee IS NOT NULL OR assignee_principal IS NOT NULL);
