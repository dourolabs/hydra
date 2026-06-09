-- baseline-version: 20260612000000
-- SQLite pre-create-statuses baseline. INSERTs are valid against the
-- schema state at sqlite migration
-- `20260612000000_issues_v2_project_id_not_null.sql`, immediately
-- before `20260613000000_create_statuses.sql` creates the new
-- `statuses` table and `20260613010000_add_issues_v2_status_sequence.sql`
-- backfills the new column on `issues_v2`. Sister to
-- `migration_baselines/20260612000000__pre_create_statuses.sql` (the
-- Postgres baseline) — kept independent so backend-specific fixture
-- changes don't ripple.
--
-- Scope: per [[i-jvmpqwwe]] acceptance criteria (b) and (c), the new
-- migrations must be exercised against:
--
--   * A custom project with statuses that carry `on_enter`,
--     `prompt_path`, and `interactive: true` so the new `statuses`
--     backfill is verified against the full column shape.
--   * One issue per DefaultProject status (`open`, `in-progress`,
--     `closed`, `dropped`, `failed`) so the `(project_id, status_key)
--     → sequence` join in the sibling migration covers every default
--     status sequence.
--   * One issue in a custom project's custom status so the same join
--     is exercised across project boundaries.
--
-- SQLite differences vs. the postgres baseline:
--   * Boolean column (`deleted`, `is_latest`) values are INTEGERs
--     (0/1) and have no trigger to backfill `is_latest`; we set it
--     explicitly.
--   * Statuses are stored as JSON TEXT, not `jsonb`.

INSERT INTO projects (
    id,
    version_number,
    key,
    name,
    statuses,
    creator,
    deleted,
    actor,
    prompt_path,
    is_latest
)
VALUES (
    'j-stsfixt',
    1,
    'stsfixt',
    'Statuses Fixture',
    '[{"key":"draft","label":"Draft","color":"#cccccc","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"reviewing","label":"Reviewing","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false,"on_enter":{"assign_to":{"Agent":{"name":"reviewer"}}},"prompt_path":"/projects/stsfixt/reviewing.md","interactive":true},{"key":"merged","label":"Merged","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}]',
    'jayantk',
    0,
    NULL,
    NULL,
    1
);

-- One issue per default-project status, plus one issue in the custom
-- project's `reviewing` status. Each row carries an explicit `status`
-- and `project_id` so the
-- `20260613010000_add_issues_v2_status_sequence.sql` backfill resolves
-- `(project_id, status) → sequence` via the new `statuses` table.
INSERT INTO issues_v2 (
    id, version_number, issue_type, description, creator, is_latest, project_id, status
) VALUES
    ('i-stsopena', 1, 'task', 'fixture: default-project status=open', 'jayantk', 1, 'j-defaul', 'open'),
    ('i-stsiprog', 1, 'task', 'fixture: default-project status=in-progress', 'jayantk', 1, 'j-defaul', 'in-progress'),
    ('i-stsclosd', 1, 'task', 'fixture: default-project status=closed', 'jayantk', 1, 'j-defaul', 'closed'),
    ('i-stsdropd', 1, 'task', 'fixture: default-project status=dropped', 'jayantk', 1, 'j-defaul', 'dropped'),
    ('i-stsfaild', 1, 'task', 'fixture: default-project status=failed', 'jayantk', 1, 'j-defaul', 'failed'),
    ('i-stsrevwg', 1, 'task', 'fixture: custom-project status=reviewing', 'jayantk', 1, 'j-stsfixt', 'reviewing');
