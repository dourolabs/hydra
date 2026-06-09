-- Reserve the `[a-z]-...` shape (a single ASCII lowercase letter
-- followed by `-`) for HydraId values. After this migration:
--
--   * The `ProjectKey` / `StatusKey` validator
--     (`hydra-common::api::v1::projects::validate_key`) rejects
--     reserved-shape values at `try_new` / `FromStr` /
--     `Deserialize`. This SQL pass rewrites the only two columns
--     that store user-chosen keys —  `metis.projects.key` and
--     `metis.statuses.key` — so they all deserialize through the
--     tightened validator.
--   * `issues_v2.project_id` stores ids (which already match the
--     reserved shape by design) — unchanged.
--   * After [[i-bqimglba]] cutover, `issues_v2.status_sequence`
--     references statuses by `(project_id, sequence)`, not by status
--     key — unchanged.
--
-- ## Rewrite policy
--
-- For every row in `metis.projects` whose `key` matches `^[a-z]-.*`
-- (Postgres regex), the new key is:
--   1. `'renamed-' || key` (the deterministic baseline), or
--   2. `'renamed-' || key || '-' || id` when (1) collides with a
--      DIFFERENT project's active-latest row (the
--      `projects_key_unique_active_idx` scope). `id` is the
--      `HydraId`, unique per project — so the suffix gives a
--      deterministic, collision-free disambiguator without a counter.
--
-- For every row in `metis.statuses` whose `key` matches `^[a-z]-.*`:
--   1. `'renamed-' || key`, or
--   2. `'renamed-' || key || '-seq' || sequence` when (1) collides
--      with another row in the same `(project_id)` (the
--      `statuses_project_key_idx` scope). `sequence` is the per-project
--      high-water-mark id from [[i-bqimglba]] and is unique within a
--      project — same determinism + collision-freedom property.
--
-- ## Idempotency
--
-- The rewrite WHERE clauses (`key ~ '^[a-z]-.*'`) match nothing after
-- the first pass: every rewritten value starts with `renamed-` (so
-- the second byte is `e`, not `-`) and therefore fails the predicate.
-- A second invocation of the body is a no-op. The
-- collision-disambiguator branch can never re-fire because its
-- output also starts with `renamed-`.
--
-- ## Logging
--
-- One `RAISE NOTICE 'projects: <id> key <old> -> <new>'` per
-- rewritten project row, and one
-- `RAISE NOTICE 'statuses: <project_id> seq <seq> key <old> -> <new>'`
-- per rewritten status row. Captured by sqlx's migration log.

DO $$
DECLARE
    rec       RECORD;
    candidate TEXT;
BEGIN
    -- Projects. Iterate in (id, version_number) order so the rewrite
    -- is deterministic regardless of the planner's row order.
    FOR rec IN
        SELECT id, version_number, key
          FROM metis.projects
         WHERE key ~ '^[a-z]-.*'
         ORDER BY id, version_number
    LOOP
        candidate := 'renamed-' || rec.key;
        IF EXISTS (
            SELECT 1
              FROM metis.projects p2
             WHERE p2.key = candidate
               AND p2.is_latest = TRUE
               AND NOT p2.deleted
               AND p2.id <> rec.id
        ) THEN
            candidate := candidate || '-' || rec.id;
        END IF;
        UPDATE metis.projects
           SET key = candidate
         WHERE id = rec.id
           AND version_number = rec.version_number;
        RAISE NOTICE 'projects: % key % -> %', rec.id, rec.key, candidate;
    END LOOP;

    -- Statuses. Iterate in (project_id, sequence) order for
    -- determinism.
    FOR rec IN
        SELECT project_id, sequence, key
          FROM metis.statuses
         WHERE key ~ '^[a-z]-.*'
         ORDER BY project_id, sequence
    LOOP
        candidate := 'renamed-' || rec.key;
        IF EXISTS (
            SELECT 1
              FROM metis.statuses s2
             WHERE s2.project_id = rec.project_id
               AND s2.key = candidate
               AND s2.sequence <> rec.sequence
        ) THEN
            candidate := candidate || '-seq' || rec.sequence;
        END IF;
        UPDATE metis.statuses
           SET key = candidate
         WHERE project_id = rec.project_id
           AND sequence = rec.sequence;
        RAISE NOTICE 'statuses: % seq % key % -> %',
            rec.project_id, rec.sequence, rec.key, candidate;
    END LOOP;
END $$;
