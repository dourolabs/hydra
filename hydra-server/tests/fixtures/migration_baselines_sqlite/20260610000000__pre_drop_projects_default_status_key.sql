-- baseline-version: 20260610000000
-- SQLite pre-drop-projects-default-status-key baseline. INSERTs are
-- valid against the schema state at sqlite migration
-- `20260610000000_add_projects_priority.sql`, immediately before
-- `20260611000000_drop_projects_default_status_key.sql` removes the
-- `default_status_key` column from `projects`. Sister to
-- `migration_baselines/20260610000000__pre_drop_projects_default_status_key.sql`.
--
-- The fixture seeds a custom project row whose `default_status_key`
-- column is populated. After roll-forward the column is gone, so the
-- typed `SqliteStore::get_project(j-dskdrop)` call must still
-- deserialize through `serde_json::from_str` without the field.

INSERT INTO projects (
    id,
    version_number,
    key,
    name,
    default_status_key,
    statuses,
    creator,
    deleted,
    actor,
    prompt_path,
    is_latest
)
VALUES (
    'j-dskdrop',
    1,
    'dskdrop',
    'Default-Status-Key Drop Fixture',
    'doing',
    '[{"key":"todo","label":"Todo","color":"#abcdef","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"doing","label":"Doing","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"done","label":"Done","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}]',
    'jayantk',
    0,
    NULL,
    '/projects/dskdrop/prompt.md',
    1
);
