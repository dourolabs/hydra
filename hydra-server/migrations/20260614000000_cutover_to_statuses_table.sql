-- PR 2 of the statuses-table cutover. Builds on PR 1's
-- `20260613000000_create_statuses.sql` and
-- `20260613010000_add_issues_v2_status_sequence.sql`, which added the
-- new `metis.statuses` table and the nullable
-- `metis.issues_v2.status_sequence` column.
--
-- This migration is a single atomic cutover from the legacy
-- `metis.projects.statuses` JSONB / `metis.issues_v2.status` TEXT shape
-- to the new `metis.statuses` table + `status_sequence` storage
-- identity. After it runs:
--
--   * `metis.projects.statuses` (JSONB) is gone.
--   * `metis.issues_v2.status` (TEXT) is gone.
--   * `metis.issues_v2.status_sequence` is NOT NULL with a FK to
--     `metis.statuses(project_id, sequence)`.
--   * `metis.projects` carries a `next_status_sequence` high-water-mark
--     column so a status `add → remove → add` cycle never reuses a
--     freed sequence id (the FK alone would allow it; the high-water
--     mark forbids it).
--
-- No dual-write phase. See [[i-djagsgtj]] for the design notes and the
-- explicit user feedback against dual-writing on the parent
-- [[i-bqimglba]].

-- 1. Catch up `metis.statuses` for any projects created in the
--    deploy gap between PR 1 merging and this migration deploying.
--    Verbatim PR 1's INSERT body with `ON CONFLICT ... DO NOTHING`, so
--    projects already backfilled by PR 1 are untouched and projects
--    inserted between then and now (whose JSONB statuses were never
--    mirrored into `metis.statuses`) get caught up here.
INSERT INTO metis.statuses (
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
    elem.ord,
    elem.value->>'key',
    elem.value->>'label',
    elem.value->>'color',
    (elem.value->>'unblocks_parents')::boolean,
    (elem.value->>'unblocks_dependents')::boolean,
    (elem.value->>'cascades_to_children')::boolean,
    NULLIF(elem.value->'on_enter', 'null'::jsonb),
    elem.value->>'prompt_path',
    COALESCE((elem.value->>'interactive')::boolean, FALSE)
FROM metis.projects p,
     jsonb_array_elements(p.statuses) WITH ORDINALITY AS elem(value, ord)
WHERE p.is_latest = TRUE
ON CONFLICT (project_id, sequence) DO NOTHING;

-- 2. Catch up `metis.issues_v2.status_sequence` for any issues
--    inserted in the deploy gap with `status TEXT` set but
--    `status_sequence` still NULL. Verbatim PR 1's body with the
--    `status_sequence IS NULL` guard, so previously-backfilled rows are
--    untouched.
UPDATE metis.issues_v2
   SET status_sequence = s.sequence
  FROM metis.statuses s
 WHERE s.project_id            = metis.issues_v2.project_id
   AND s.key                   = metis.issues_v2.status
   AND metis.issues_v2.status_sequence IS NULL;

-- 3. Pre-flight NULL guard. Mirrors PR 1's tail guard. If any issue
--    row still has NULL `status_sequence`, abort the migration loudly
--    rather than silently letting the `SET NOT NULL` step error with a
--    cryptic constraint violation.
DO $$
DECLARE
    null_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO null_count
      FROM metis.issues_v2
     WHERE status_sequence IS NULL;
    IF null_count > 0 THEN
        RAISE EXCEPTION
            'cutover_to_statuses_table: refusing to complete; % NULL status_sequence row(s) remain after catch-up backfill. Inspect orphan (project_id, status) pairs in metis.issues_v2 that have no matching metis.statuses row.',
            null_count;
    END IF;
END $$;

-- 4. Per-project high-water-mark column for sequence assignment.
--    Monotonically non-decreasing across status add/remove cycles to
--    forbid sequence id reuse: `add_status` reads + increments this
--    column atomically; `remove_status` leaves it untouched. The
--    backfill seeds each project to one past its current max sequence,
--    or 1 when the project has no statuses yet.
ALTER TABLE metis.projects
    ADD COLUMN next_status_sequence BIGINT NOT NULL DEFAULT 1;

UPDATE metis.projects p
   SET next_status_sequence = COALESCE(
       (SELECT MAX(sequence) + 1 FROM metis.statuses WHERE project_id = p.id),
       1
   );

-- 5. Tighten `status_sequence` to NOT NULL now that every row is
--    backfilled (guard above) and the store layer of this PR writes it
--    on every insert.
ALTER TABLE metis.issues_v2 ALTER COLUMN status_sequence SET NOT NULL;

-- 6. FK from `issues_v2 → statuses` on `(project_id, status_sequence)`.
--    `ON DELETE RESTRICT, ON UPDATE RESTRICT` are the postgres defaults
--    but stated explicitly: the whole point of this FK is to prevent
--    orphaning, so the policy must be obvious at the SQL level.
ALTER TABLE metis.issues_v2
    ADD CONSTRAINT issues_v2_status_sequence_fkey
    FOREIGN KEY (project_id, status_sequence)
    REFERENCES metis.statuses(project_id, sequence)
    ON DELETE RESTRICT ON UPDATE RESTRICT;

-- 7. Drop the legacy `status TEXT` column. After this, every
--    application-level read of an issue's status comes from a JOIN to
--    `metis.statuses` on `(project_id, status_sequence)`.
ALTER TABLE metis.issues_v2 DROP COLUMN status;

-- 8. Drop the legacy `statuses` JSONB column. `metis.statuses` is now
--    the sole source of truth for per-project status definitions.
ALTER TABLE metis.projects DROP COLUMN statuses;
