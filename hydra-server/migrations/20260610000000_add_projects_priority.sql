-- Add an explicit `priority` sort key to `metis.projects` so the order
-- of projects in `list_projects` is user-controlled and deterministic
-- instead of "whichever project's latest version row was written most
-- recently". See the SQLite sibling migration of the same date for the
-- full design discussion; this file mirrors it for the Postgres backend.
--
-- Backfill rule: rank the latest-version rows by today's effective
-- ordering (`created_at DESC, id DESC`) and assign `priority = rn *
-- 1000.0` so existing deployments preserve the user-visible order at
-- migration-apply time.
ALTER TABLE metis.projects ADD COLUMN priority DOUBLE PRECISION NOT NULL DEFAULT 0.0;

WITH ranked AS (
    SELECT id, version_number,
           ROW_NUMBER() OVER (ORDER BY created_at DESC, id DESC) AS rn
    FROM metis.projects
    WHERE is_latest = true
)
UPDATE metis.projects p
   SET priority = r.rn * 1000.0
  FROM ranked r
 WHERE p.id             = r.id
   AND p.version_number = r.version_number;
