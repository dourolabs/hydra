-- baseline-version: 20260613020000
-- Postgres pre-cutover baseline. INSERTs are valid against the schema
-- state after sqlite migration
-- `20260613010000_add_issues_v2_status_sequence.sql` and immediately
-- before `20260614000000_cutover_to_statuses_table.sql` drops the
-- legacy `projects.statuses` JSONB / `issues_v2.status` TEXT columns,
-- tightens `issues_v2.status_sequence` to NOT NULL, adds the FK, and
-- introduces `metis.projects.next_status_sequence`. See [[i-djagsgtj]]
-- for design.
--
-- Scope: per [[i-djagsgtj]] acceptance criteria, the cutover migration
-- must be exercised against:
--
--   * A custom project whose `metis.statuses` rows are already
--     populated (the steady-state shape that PR 1 backfilled). Issues
--     in it carry both `status TEXT` and `status_sequence`.
--   * A **deploy-gap project** inserted AFTER PR 1 was applied but
--     BEFORE this cutover migration was applied: `projects.statuses`
--     JSONB is populated but `metis.statuses` rows are absent, and at
--     least one issue in it has `status TEXT` set with
--     `status_sequence = NULL`. The cutover migration must backfill
--     both before tightening the column.
--   * A **deploy-gap issue** in an already-migrated project: same
--     condition (NULL `status_sequence`), to exercise the
--     issue-side catch-up in isolation.
--
-- The seeded `j-defaul` row + the prior `j-stsfixt` baseline row from
-- `20260612000000__pre_create_statuses.sql` together with PR 1's
-- backfill provide the steady-state coverage; the rows below add the
-- deploy-gap coverage on top.

-- Steady-state custom project (no deploy gap). `metis.statuses` rows
-- for this project were inserted by PR 1's backfill of the JSONB
-- column; no additional seed needed beyond the JSONB row itself.
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
    'j-cutsteady',
    1,
    'cutsteady',
    'Cutover Steady-State',
    '[
        {"key":"queued","label":"Queued","color":"#aaaaaa","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"shipped","label":"Shipped","color":"#22aa22","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}
    ]'::jsonb,
    'jayantk',
    NULL
);

-- Catch up `metis.statuses` for the steady-state project — simulating
-- the situation where PR 1 had already been applied and the rows are
-- present. The cutover's catch-up INSERT must NOT duplicate these.
INSERT INTO metis.statuses (
    project_id, sequence, key, label, color,
    unblocks_parents, unblocks_dependents, cascades_to_children,
    on_enter, prompt_path, interactive
) VALUES
    ('j-cutsteady', 1, 'queued',  'Queued',  '#aaaaaa', FALSE, FALSE, FALSE, NULL, NULL, FALSE),
    ('j-cutsteady', 2, 'shipped', 'Shipped', '#22aa22', TRUE,  TRUE,  FALSE, NULL, NULL, FALSE);

-- A deploy-gap project: JSONB statuses are set but no `metis.statuses`
-- rows exist for it. The cutover's INSERT must populate them.
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
    'j-cutgapprj',
    1,
    'cutgapprj',
    'Cutover Deploy-Gap Project',
    '[
        {"key":"intake","label":"Intake","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"done","label":"Done","color":"#27ae60","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false,"on_enter":{"assign_to":{"Agent":{"name":"reviewer"}}},"prompt_path":"/projects/cutgapprj/done.md","interactive":true}
    ]'::jsonb,
    'jayantk',
    NULL
);

-- Issues that exercise the catch-up paths. Each is on `is_latest = TRUE`
-- via the BEFORE-INSERT trigger.
INSERT INTO metis.issues_v2 (
    id, version_number, issue_type, description, creator, project_id, status, status_sequence
) VALUES
    -- Steady-state: both columns populated. Cutover preserves
    -- status_sequence and drops the TEXT column.
    ('i-cutsteadya', 1, 'task', 'fixture: steady status=queued',  'jayantk', 'j-cutsteady', 'queued',  1),
    ('i-cutsteadyb', 1, 'task', 'fixture: steady status=shipped', 'jayantk', 'j-cutsteady', 'shipped', 2),
    -- Deploy-gap project issue: status TEXT populated but
    -- status_sequence NULL because the issue was inserted while the
    -- catch-up paths were quiescent. Cutover step 2 backfills it.
    ('i-cutgapa',    1, 'task', 'fixture: gap-project status=intake', 'jayantk', 'j-cutgapprj', 'intake', NULL),
    -- Deploy-gap issue in already-migrated project (j-defaul has
    -- statuses 1..=5 populated by PR 1). status_sequence is NULL until
    -- the cutover backfills.
    ('i-cutgapdef',  1, 'task', 'fixture: gap-issue on j-defaul',     'jayantk', 'j-defaul', 'open', NULL);
