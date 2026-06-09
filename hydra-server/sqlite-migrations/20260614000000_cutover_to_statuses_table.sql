-- Sister to the Postgres `20260614000000_cutover_to_statuses_table.sql`.
-- Single atomic cutover from the legacy `projects.statuses` JSON TEXT
-- and `issues_v2.status` TEXT shape to the new `statuses` table +
-- `status_sequence` storage identity. Builds on PR 1's
-- `20260613000000_create_statuses.sql` and
-- `20260613010000_add_issues_v2_status_sequence.sql`. See
-- [[i-djagsgtj]] for the design and [[i-bqimglba]] for the parent.
--
-- After this migration runs:
--   * `projects.statuses` (JSON TEXT) is gone.
--   * `issues_v2.status` (TEXT) is gone.
--   * `issues_v2.status_sequence` is NOT NULL with a FK to
--     `statuses(project_id, sequence)`.
--   * `projects` carries a `next_status_sequence` high-water-mark
--     column so a status add → remove → add cycle never reuses a freed
--     sequence id.
--
-- SQLite differences vs. the postgres sibling:
--   * No `metis.` schema prefix.
--   * BOOLEAN columns are INTEGER (0/1); BIGINT becomes INTEGER.
--   * No `ALTER COLUMN ... SET NOT NULL` and no `ADD CONSTRAINT FK`
--     against an existing table — use the table-rebuild dance for both
--     `projects` (drop `statuses` JSON TEXT column) and `issues_v2`
--     (drop `status` TEXT column, tighten `status_sequence` NOT NULL,
--     add FK to `statuses`).
--   * `RAISE EXCEPTION` becomes a CHECK constraint on a scratch table,
--     same shape as PR 1's NULL guard.
--   * Per the SQLite column-reorder memory rule, every table-rebuild
--     INSERT names columns explicitly in both INSERT and SELECT.

-- 1. Catch-up backfill into `statuses` for any projects whose rows
--    were inserted in the deploy gap between PR 1 merging and this
--    migration deploying. `INSERT OR IGNORE` makes already-backfilled
--    rows a no-op.
--    Note: catch-up is insert-only. Modifications to
--    `projects.statuses` JSON during the deploy gap (label/color
--    edits, key renames, reorders, status removal) are not
--    reconciled — the existing `statuses` row wins. The step-3 NULL
--    guard catches the worst case (orphaned issue) cleanly.
INSERT OR IGNORE INTO statuses (
    project_id,
    sequence,
    key,
    label,
    color,
    unblocks_parents,
    unblocks_dependents,
    cascades_to_children,
    on_enter,
    prompt_path,
    interactive
)
SELECT
    p.id,
    elem.key + 1,
    json_extract(elem.value, '$.key'),
    json_extract(elem.value, '$.label'),
    json_extract(elem.value, '$.color'),
    json_extract(elem.value, '$.unblocks_parents'),
    json_extract(elem.value, '$.unblocks_dependents'),
    json_extract(elem.value, '$.cascades_to_children'),
    json_extract(elem.value, '$.on_enter'),
    json_extract(elem.value, '$.prompt_path'),
    COALESCE(json_extract(elem.value, '$.interactive'), 0)
FROM projects p, json_each(p.statuses) elem
WHERE p.is_latest = 1;

-- 2. Catch-up backfill `issues_v2.status_sequence` for any issues
--    inserted post-PR 1 with `status TEXT` set but `status_sequence`
--    still NULL. Same correlated sub-SELECT pattern PR 1 used; the
--    `WHERE status_sequence IS NULL` guard makes this idempotent.
UPDATE issues_v2
   SET status_sequence = (
       SELECT s.sequence
         FROM statuses s
        WHERE s.project_id = issues_v2.project_id
          AND s.key        = issues_v2.status
   )
 WHERE status_sequence IS NULL;

-- 3. Pre-flight NULL guard. SQLite cannot `RAISE` outside trigger
--    bodies, so the count of remaining NULL rows is funnelled into a
--    scratch table whose CHECK trips the migration on any non-zero
--    count. The DROP TABLE IF EXISTS at the head clears any leftover
--    from a previously-aborted migration.
DROP TABLE IF EXISTS _cutover_null_guard;
CREATE TABLE _cutover_null_guard (
    null_count INTEGER NOT NULL CHECK (null_count = 0)
);
INSERT INTO _cutover_null_guard (null_count)
    SELECT COUNT(*) FROM issues_v2 WHERE status_sequence IS NULL;
DROP TABLE _cutover_null_guard;

-- 4. Add the per-project high-water-mark column. Monotonically
--    non-decreasing across status add/remove cycles to forbid
--    sequence id reuse: `add_status` reads + increments this column
--    atomically; `remove_status` leaves it untouched. The backfill
--    seeds each project to one past its current max sequence, or 1
--    when the project has no statuses yet.
ALTER TABLE projects ADD COLUMN next_status_sequence INTEGER NOT NULL DEFAULT 1;

UPDATE projects
   SET next_status_sequence = COALESCE(
       (SELECT MAX(s.sequence) + 1 FROM statuses s WHERE s.project_id = projects.id),
       1
   );

-- 5. Rebuild `projects` to drop the `statuses` JSON TEXT column.
--    SQLite ≥ 3.35 supports `ALTER TABLE ... DROP COLUMN` but the
--    table-rebuild dance is what the rest of this file's migrations
--    use; staying consistent. The rebuild lists columns by name
--    explicitly on both sides to keep the migration-framework test
--    re-run idempotent (per the SQLite column-reorder memory rule).
CREATE TABLE projects_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    key TEXT NOT NULL,
    name TEXT NOT NULL,
    creator TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    prompt_path TEXT DEFAULT NULL,
    priority REAL NOT NULL DEFAULT 0.0,
    next_status_sequence INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (id, version_number)
);

INSERT INTO projects_new (
    id, version_number, key, name, creator, deleted, actor,
    is_latest, created_at, updated_at, prompt_path, priority,
    next_status_sequence
)
SELECT
    id, version_number, key, name, creator, deleted, actor,
    is_latest, created_at, updated_at, prompt_path, priority,
    next_status_sequence
FROM projects;

DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;

CREATE INDEX IF NOT EXISTS projects_latest_idx ON projects (id, version_number DESC);
CREATE INDEX IF NOT EXISTS projects_creator_idx ON projects (creator) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS projects_is_latest_idx ON projects (id) WHERE is_latest = 1;
CREATE UNIQUE INDEX IF NOT EXISTS projects_key_unique_active_idx
    ON projects (key) WHERE is_latest = 1 AND deleted = 0;

-- 6. Rebuild `issues_v2` to drop the `status TEXT` column, tighten
--    `status_sequence` to NOT NULL, and add the FK against
--    `statuses(project_id, sequence)`. SQLite enforces the FK
--    per-statement when `PRAGMA foreign_keys=ON`; this migration body
--    runs inside a sqlx-managed transaction and the backfill in step 2
--    plus the guard in step 3 together ensure every row that's about
--    to be copied has a matching `(project_id, status_sequence)` in
--    `statuses`, so the FK validates immediately on each INSERT.
--    `ON DELETE RESTRICT ON UPDATE RESTRICT` is stated explicitly even
--    though it's the SQLite default — the whole point of this FK is to
--    prevent orphaning.
CREATE TABLE issues_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    issue_type TEXT NOT NULL,
    description TEXT NOT NULL,
    creator TEXT NOT NULL,
    progress TEXT NOT NULL DEFAULT '',
    status_sequence INTEGER NOT NULL,
    assignee TEXT,
    job_settings TEXT NOT NULL DEFAULT '{}',
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    form TEXT DEFAULT NULL,
    form_response TEXT DEFAULT NULL,
    feedback TEXT DEFAULT NULL,
    is_latest INTEGER NOT NULL DEFAULT 0,
    assignee_principal TEXT,
    project_id TEXT NOT NULL,
    PRIMARY KEY (id, version_number),
    FOREIGN KEY (project_id, status_sequence)
        REFERENCES statuses(project_id, sequence)
        ON DELETE RESTRICT ON UPDATE RESTRICT
);

INSERT INTO issues_v2_new (
    id, version_number, title, issue_type, description, creator, progress,
    status_sequence, assignee, job_settings, deleted, actor, created_at, updated_at,
    form, form_response, feedback, is_latest, assignee_principal, project_id
)
SELECT
    id, version_number, title, issue_type, description, creator, progress,
    status_sequence, assignee, job_settings, deleted, actor, created_at, updated_at,
    form, form_response, feedback, is_latest, assignee_principal, project_id
FROM issues_v2;

DROP TABLE issues_v2;
ALTER TABLE issues_v2_new RENAME TO issues_v2;

-- Recreate every index that lived on the old `issues_v2` table. The
-- previous `issues_v2_status_idx` on the dropped `status` column is
-- replaced by `issues_v2_project_status_sequence_idx`, already
-- recreated below from PR 1.
CREATE INDEX IF NOT EXISTS issues_v2_latest_idx ON issues_v2 (id, version_number DESC);
CREATE INDEX IF NOT EXISTS issues_v2_latest_id_idx ON issues_v2 (id) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS issues_v2_latest_pagination_idx ON issues_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS issues_v2_project_id_idx ON issues_v2 (project_id);
CREATE INDEX IF NOT EXISTS issues_v2_updated_at_id_idx ON issues_v2 (updated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS issues_v2_project_status_sequence_idx
    ON issues_v2 (project_id, status_sequence);
