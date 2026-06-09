-- baseline-version: 20260612000000
-- Postgres pre-create-statuses baseline. INSERTs are valid against the
-- schema state at postgres migration
-- `20260612000000_issues_v2_project_id_not_null.sql`, immediately
-- before `20260613000000_create_statuses.sql` creates the new
-- `metis.statuses` table and `20260613010000_add_issues_v2_status_sequence.sql`
-- backfills the new column on `metis.issues_v2`. Sister to
-- `migration_baselines_sqlite/20260612000000__pre_create_statuses.sql`
-- (the SQLite baseline) — kept independent so backend-specific fixture
-- changes don't ripple.
--
-- Scope: per [[i-jvmpqwwe]] acceptance criteria (b) and (c), the new
-- migrations must be exercised against:
--
--   * A custom project with statuses that carry `on_enter`,
--     `prompt_path`, and `interactive: true` so the `metis.statuses`
--     backfill is verified against the full column shape (not just
--     the trivial default-project columns).
--   * One issue per DefaultProject status (`open`, `in-progress`,
--     `closed`, `dropped`, `failed`) so the `(project_id, status_key)
--     → sequence` join in the sibling migration covers every default
--     status sequence.
--   * One issue in a custom project's custom status so the same join
--     is exercised across project boundaries.
--
-- The seeded `j-defaul` row carries 5 statuses already
-- (open / in-progress / closed / dropped / failed); the
-- `20260613000000_create_statuses.sql` backfill will pick those up
-- automatically, so no `j-defaul` re-seed is needed here.

-- Custom project with full-column-shape statuses. `draft` is the
-- minimal shape; `reviewing` exercises `on_enter` (a non-NULL
-- `StatusOnEnter` blob), `prompt_path`, and `interactive: true`;
-- `merged` is a terminal status with `unblocks_*` flags set so the
-- boolean backfill is exercised for every column.
INSERT INTO metis.projects (
    id,
    version_number,
    key,
    name,
    statuses,
    creator,
    prompt_path
)
VALUES (
    'j-stsfixt',
    1,
    'stsfixt',
    'Statuses Fixture',
    '[
        {"key":"draft","label":"Draft","color":"#cccccc","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"reviewing","label":"Reviewing","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false,"on_enter":{"assign_to":{"Agent":{"name":"reviewer"}}},"prompt_path":"/projects/stsfixt/reviewing.md","interactive":true},
        {"key":"merged","label":"Merged","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}
    ]'::jsonb,
    'jayantk',
    NULL
);

-- One issue per default-project status. Each row carries an explicit
-- `status` and `project_id` so the
-- `20260613010000_add_issues_v2_status_sequence.sql` backfill resolves
-- `(j-defaul, <status>) → sequence` via the new `metis.statuses` table.
-- `is_latest` is set by the `versioned_set_is_latest` BEFORE-INSERT
-- trigger on `metis.issues_v2`; omit the column from the INSERT.
INSERT INTO metis.issues_v2 (
    id, version_number, issue_type, description, creator, project_id, status
) VALUES
    ('i-stsopena', 1, 'task', 'fixture: default-project status=open', 'jayantk', 'j-defaul', 'open'),
    ('i-stsiprog', 1, 'task', 'fixture: default-project status=in-progress', 'jayantk', 'j-defaul', 'in-progress'),
    ('i-stsclosd', 1, 'task', 'fixture: default-project status=closed', 'jayantk', 'j-defaul', 'closed'),
    ('i-stsdropd', 1, 'task', 'fixture: default-project status=dropped', 'jayantk', 'j-defaul', 'dropped'),
    ('i-stsfaild', 1, 'task', 'fixture: default-project status=failed', 'jayantk', 'j-defaul', 'failed'),
    ('i-stsrevwg', 1, 'task', 'fixture: custom-project status=reviewing', 'jayantk', 'j-stsfixt', 'reviewing');
