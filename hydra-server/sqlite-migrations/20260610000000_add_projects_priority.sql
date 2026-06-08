-- Add an explicit `priority` sort key to `projects` so the order of
-- projects in `list_projects` is user-controlled and deterministic
-- instead of "whichever project's latest version row was written most
-- recently" (every project mutation writes a new version row with a
-- fresh `created_at`, so today's `ORDER BY p.created_at DESC, p.id DESC`
-- shuffles the list whenever anything under a project is touched).
--
-- Sort direction is ascending (smaller `priority` appears earlier), with
-- `created_at DESC, id DESC` as a stable tiebreak. The Rust default for
-- new projects is `0.0` (so they sort to the top until the drag-and-drop
-- UI assigns a real value); the column default mirrors that so the
-- non-latest historical version rows we never read keep a sane value.
--
-- Backfill rule: rank the latest-version rows by today's effective
-- ordering (`created_at DESC, id DESC`) and assign `priority = rn *
-- 1000.0` so existing deployments preserve the user-visible order at
-- migration-apply time. The default project (`j-defaul`) is included in
-- the rank — its row was inserted by `20260607000000_seed_default_project`
-- with NOW() as `created_at`, so on a fresh deploy where the seed runs
-- first it lands at rn=1 → priority=1000.0, matching
-- `default_project_seed()` in domain. On an existing deploy with custom
-- projects already created, the default project (newest `created_at`)
-- still ranks first by the same logic.
ALTER TABLE projects ADD COLUMN priority REAL NOT NULL DEFAULT 0.0;

WITH ranked AS (
    SELECT id, version_number,
           ROW_NUMBER() OVER (ORDER BY created_at DESC, id DESC) AS rn
    FROM projects
    WHERE is_latest = 1
)
UPDATE projects
   SET priority = (SELECT rn * 1000.0 FROM ranked
                   WHERE ranked.id            = projects.id
                     AND ranked.version_number = projects.version_number)
 WHERE EXISTS (SELECT 1 FROM ranked
               WHERE ranked.id            = projects.id
                 AND ranked.version_number = projects.version_number);
