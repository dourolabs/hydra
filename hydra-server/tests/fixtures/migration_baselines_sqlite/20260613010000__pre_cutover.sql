-- baseline-version: 20260613020000
-- SQLite pre-cutover baseline. INSERTs are valid against the schema
-- state after sqlite migration
-- `20260613010000_add_issues_v2_status_sequence.sql` and immediately
-- before `20260614000000_cutover_to_statuses_table.sql`. Sister to
-- `migration_baselines/20260613020000__pre_cutover.sql` — kept
-- independent so backend-specific fixture changes don't ripple. See
-- [[i-djagsgtj]] for design.
--
-- Scope matches the postgres baseline:
--   * A steady-state custom project: `projects.statuses` JSON + already
--     populated `statuses` rows + issues with both `status` TEXT and
--     `status_sequence` set.
--   * A deploy-gap project: JSON populated but `statuses` rows missing,
--     and at least one issue with `status_sequence` NULL.
--   * A deploy-gap issue in an already-migrated project (`j-defaul`)
--     with `status_sequence` NULL.

INSERT INTO projects (
    id, version_number, key, name, statuses, creator,
    deleted, actor, prompt_path, is_latest
)
VALUES (
    'j-cutsteady',
    1,
    'cutsteady',
    'Cutover Steady-State',
    '[{"key":"queued","label":"Queued","color":"#aaaaaa","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"shipped","label":"Shipped","color":"#22aa22","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}]',
    'jayantk',
    0,
    NULL,
    NULL,
    1
);

INSERT INTO statuses (
    project_id, sequence, key, label, color,
    unblocks_parents, unblocks_dependents, cascades_to_children,
    on_enter, prompt_path, interactive
) VALUES
    ('j-cutsteady', 1, 'queued',  'Queued',  '#aaaaaa', 0, 0, 0, NULL, NULL, 0),
    ('j-cutsteady', 2, 'shipped', 'Shipped', '#22aa22', 1, 1, 0, NULL, NULL, 0);

-- Deploy-gap project: JSON populated, statuses rows absent.
INSERT INTO projects (
    id, version_number, key, name, statuses, creator,
    deleted, actor, prompt_path, is_latest
)
VALUES (
    'j-cutgapprj',
    1,
    'cutgapprj',
    'Cutover Deploy-Gap Project',
    '[{"key":"intake","label":"Intake","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"done","label":"Done","color":"#27ae60","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false,"on_enter":{"assign_to":{"Agent":{"name":"reviewer"}}},"prompt_path":"/projects/cutgapprj/done.md","interactive":true}]',
    'jayantk',
    0,
    NULL,
    NULL,
    1
);

-- Issues. SQLite has no BEFORE-INSERT trigger for `is_latest`, so set
-- it explicitly. `status` TEXT is populated for every row;
-- `status_sequence` is NULL for the deploy-gap rows so the catch-up
-- backfill in step 2 of the cutover migration is exercised.
INSERT INTO issues_v2 (
    id, version_number, issue_type, description, creator, is_latest,
    project_id, status, status_sequence
) VALUES
    ('i-cutsteadya', 1, 'task', 'fixture: steady status=queued',      'jayantk', 1, 'j-cutsteady', 'queued',  1),
    ('i-cutsteadyb', 1, 'task', 'fixture: steady status=shipped',     'jayantk', 1, 'j-cutsteady', 'shipped', 2),
    ('i-cutgapa',    1, 'task', 'fixture: gap-project status=intake', 'jayantk', 1, 'j-cutgapprj', 'intake', NULL),
    ('i-cutgapdef',  1, 'task', 'fixture: gap-issue on j-defaul',     'jayantk', 1, 'j-defaul',    'open',   NULL);
