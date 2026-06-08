-- baseline-version: 20260610000000
-- Postgres pre-drop-projects-default-status-key baseline. INSERTs are
-- valid against the schema state at postgres migration
-- `20260610000000_add_projects_priority.sql`, immediately before
-- `20260611000000_drop_projects_default_status_key.sql` removes the
-- `default_status_key` column from `metis.projects`.
--
-- Scope: per [[i-gtkifurc]], the drop-default-status-key migration
-- needs migration-framework coverage that exercises the column drop
-- on a non-default row (the seeded `j-defaul` row is already present
-- post-rollforward via the seed migration).
--
-- The fixture seeds a custom project row whose `default_status_key`
-- column is populated. After roll-forward the column is gone, so the
-- typed `Store::get_project(j-dskdrop)` call must still deserialize
-- the row through `serde_json::from_value` without the field.

INSERT INTO metis.projects (
    id,
    version_number,
    key,
    name,
    default_status_key,
    statuses,
    creator,
    prompt_path
)
VALUES (
    'j-dskdrop',
    1,
    'dskdrop',
    'Default-Status-Key Drop Fixture',
    'doing',
    '[
        {"key":"todo","label":"Todo","color":"#abcdef","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"doing","label":"Doing","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"done","label":"Done","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}
    ]'::jsonb,
    'jayantk',
    '/projects/dskdrop/prompt.md'
);
