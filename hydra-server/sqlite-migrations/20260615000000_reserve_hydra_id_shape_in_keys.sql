-- Sister to the Postgres
-- `20260615000000_reserve_hydra_id_shape_in_keys.sql`. Reserves the
-- `[a-z]-...` shape for HydraId values by rewriting every
-- `projects.key` / `statuses.key` row whose value matches the shape.
-- See [[i-njohfbbk]] for design + the postgres sibling for the
-- rewrite-policy and idempotency discussion.
--
-- SQLite differences vs. the postgres sibling:
--   * No `metis.` schema prefix.
--   * No PL/pgSQL — the rewrite plan is materialized into a scratch
--     `_kshape_renames_*` table, the UPDATE reads its `new_key` per
--     row, and a final `SELECT printf(...)` emits the audit log lines
--     for sqlx to capture.
--   * `^[a-z]-.*` regex becomes the equivalent `key GLOB '[a-z]-*'`.

-- 1. Project rewrites. The plan table captures (id, version_number,
--    old_key, new_key) for every row matching the reserved shape.
--    `new_key` evaluates the collision-disambiguator branch:
--    `renamed-<key>` baseline, falling back to
--    `renamed-<key>-<id>` when another DIFFERENT-id active-latest
--    row already holds the candidate key.

DROP TABLE IF EXISTS _kshape_renames_projects;
CREATE TABLE _kshape_renames_projects (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    old_key TEXT NOT NULL,
    new_key TEXT NOT NULL,
    PRIMARY KEY (id, version_number)
);

INSERT INTO _kshape_renames_projects (id, version_number, old_key, new_key)
SELECT
    p.id,
    p.version_number,
    p.key,
    CASE
        WHEN EXISTS (
            SELECT 1
              FROM projects p2
             WHERE p2.key = 'renamed-' || p.key
               AND p2.is_latest = 1
               AND p2.deleted = 0
               AND p2.id <> p.id
        )
        THEN 'renamed-' || p.key || '-' || p.id
        ELSE 'renamed-' || p.key
    END
FROM projects p
WHERE p.key GLOB '[a-z]-*'
ORDER BY p.id, p.version_number;

UPDATE projects
   SET key = (
       SELECT new_key
         FROM _kshape_renames_projects r
        WHERE r.id = projects.id
          AND r.version_number = projects.version_number
   )
 WHERE EXISTS (
       SELECT 1
         FROM _kshape_renames_projects r
        WHERE r.id = projects.id
          AND r.version_number = projects.version_number
   );

-- 2. Status rewrites. Plan + apply identical in shape; collision
--    scope is `(project_id)` (the `statuses_project_key_idx`
--    uniqueness scope), disambiguator is the per-project `sequence`.

DROP TABLE IF EXISTS _kshape_renames_statuses;
CREATE TABLE _kshape_renames_statuses (
    project_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    old_key TEXT NOT NULL,
    new_key TEXT NOT NULL,
    PRIMARY KEY (project_id, sequence)
);

INSERT INTO _kshape_renames_statuses (project_id, sequence, old_key, new_key)
SELECT
    s.project_id,
    s.sequence,
    s.key,
    CASE
        WHEN EXISTS (
            SELECT 1
              FROM statuses s2
             WHERE s2.project_id = s.project_id
               AND s2.key = 'renamed-' || s.key
               AND s2.sequence <> s.sequence
        )
        THEN 'renamed-' || s.key || '-seq' || s.sequence
        ELSE 'renamed-' || s.key
    END
FROM statuses s
WHERE s.key GLOB '[a-z]-*'
ORDER BY s.project_id, s.sequence;

UPDATE statuses
   SET key = (
       SELECT new_key
         FROM _kshape_renames_statuses r
        WHERE r.project_id = statuses.project_id
          AND r.sequence = statuses.sequence
   )
 WHERE EXISTS (
       SELECT 1
         FROM _kshape_renames_statuses r
        WHERE r.project_id = statuses.project_id
          AND r.sequence = statuses.sequence
   );

-- 3. Emit the audit lines. sqlx's migration runner pulls SELECT
--    output through to the log, matching the postgres `RAISE NOTICE`
--    audit behaviour. Same format on both backends.
SELECT printf('projects: %s key %s -> %s', id, old_key, new_key)
  FROM _kshape_renames_projects
 ORDER BY id, version_number;
SELECT printf('statuses: %s seq %d key %s -> %s', project_id, sequence, old_key, new_key)
  FROM _kshape_renames_statuses
 ORDER BY project_id, sequence;

DROP TABLE _kshape_renames_projects;
DROP TABLE _kshape_renames_statuses;
