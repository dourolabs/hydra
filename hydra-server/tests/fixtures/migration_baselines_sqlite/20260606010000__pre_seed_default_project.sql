-- baseline-version: 20260606010000
-- SQLite pre-seed-default-project baseline. INSERTs are valid against the
-- schema state at sqlite migration `20260606010000_add_projects_prompt_path.sql`,
-- immediately before `20260607000000_seed_default_project.sql` inserts the
-- 'j-defaul' projects row and backfills `issues_v2.project_id`. Sister to
-- `migration_baselines/20260606010000__pre_seed_default_project.sql` (the
-- postgres baseline) — kept independent so postgres-only fixture changes
-- don't ripple here.
--
-- Scope: per [[i-bivbnsgb]], the seed_default_project migration
-- (introduced by [[p-xtixlxfy]]) had no coverage under the existing
-- migration-test framework. This baseline pre-seeds `issues_v2` rows
-- with NULL `project_id`, including multi-version rows of the same
-- logical issue, so the test can assert the backfill UPDATE touches
-- every NULL row regardless of version.
--
-- SQLite differences vs. the postgres baseline:
--   * No `metis.` schema prefix.
--   * Boolean columns (`is_latest`) are INTEGERs (0/1) and have no
--     trigger to backfill `is_latest`; we set it explicitly.

-- Single-version issue with NULL project_id (the column was added by
-- 20260604000001_create_projects.sql without a default, so this row's
-- project_id stays NULL until the seed migration's backfill runs).
INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, is_latest)
VALUES
    ('i-seedone',  1, 'task', 'single-version row with NULL project_id', 'jayantk', 1);

-- Multi-version issue (v1, v2) of the same logical id. Both versions
-- carry NULL project_id; the seed_default_project migration's
-- `UPDATE issues_v2 SET project_id = 'j-defaul' WHERE project_id IS NULL`
-- must touch every NULL row regardless of `is_latest`.
INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, is_latest)
VALUES
    ('i-seedmv',   1, 'task', 'multi-version row v1, NULL project_id', 'jayantk', 0),
    ('i-seedmv',   2, 'task', 'multi-version row v2, NULL project_id', 'jayantk', 1);
