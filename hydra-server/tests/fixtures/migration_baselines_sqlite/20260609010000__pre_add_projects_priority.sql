-- baseline-version: 20260609010000
-- SQLite pre-add-projects-priority baseline. INSERTs are valid against
-- the schema state at sqlite migration `20260609010000_drop_actors_v2.sql`,
-- immediately before `20260610000000_add_projects_priority.sql` adds the
-- `priority REAL NOT NULL DEFAULT 0.0` column and runs the rank backfill
-- (`ROW_NUMBER() OVER (ORDER BY created_at DESC, id DESC) * 1000.0`).
-- Sister to `migration_baselines/20260609010000__pre_add_projects_priority.sql`
-- (the Postgres baseline) — kept independent so postgres-only fixture
-- changes don't ripple here.
--
-- Scope: per the parent issue (Add Project.priority, backfill, sort by
-- it), the priority migration's backfill UPDATE must be exercised
-- against pre-existing latest-version rows. The three inserted rows
-- carry explicit `created_at` values far in the future (2027-...) so
-- they sort newest-first ahead of the `j-defaul` and `j-iconfix` rows
-- seeded by prior baselines / migrations — giving the three rows
-- well-defined ranks of 1 / 2 / 3 and final priorities of
-- 1000 / 2000 / 3000 regardless of when the test runs.
--
-- SQLite differences vs. the postgres baseline:
--   * Boolean columns (`deleted`, `is_latest`) are INTEGERs (0/1) and
--     have no trigger to backfill `is_latest`; we set it explicitly.
--   * Statuses are stored as JSON TEXT, not `jsonb`.
--   * `created_at` is a TEXT timestamp string, not a `TIMESTAMPTZ`.

-- Newest → rank 1 → priority 1000.0 after backfill.
INSERT INTO projects (
    id, version_number, key, name, default_status_key, statuses,
    creator, deleted, actor, prompt_path, is_latest, created_at
)
VALUES (
    'j-prione', 1, 'priority-one', 'Priority One', 'open',
    '[{"key":"open","label":"Open","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false}]',
    'jayantk', 0, NULL, NULL, 1, '2027-01-03T00:00:00+00:00'
);

-- Middle → rank 2 → priority 2000.0 after backfill.
INSERT INTO projects (
    id, version_number, key, name, default_status_key, statuses,
    creator, deleted, actor, prompt_path, is_latest, created_at
)
VALUES (
    'j-pritwo', 1, 'priority-two', 'Priority Two', 'open',
    '[{"key":"open","label":"Open","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false}]',
    'jayantk', 0, NULL, NULL, 1, '2027-01-02T00:00:00+00:00'
);

-- Oldest → rank 3 → priority 3000.0 after backfill.
INSERT INTO projects (
    id, version_number, key, name, default_status_key, statuses,
    creator, deleted, actor, prompt_path, is_latest, created_at
)
VALUES (
    'j-pritri', 1, 'priority-three', 'Priority Three', 'open',
    '[{"key":"open","label":"Open","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false}]',
    'jayantk', 0, NULL, NULL, 1, '2027-01-01T00:00:00+00:00'
);
