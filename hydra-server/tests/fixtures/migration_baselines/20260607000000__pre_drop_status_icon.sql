-- baseline-version: 20260607000000
-- Postgres pre-drop-status-icon baseline. INSERTs are valid against the
-- schema state at postgres migration
-- `20260607000000_seed_default_project.sql`, immediately before
-- `20260608000000_drop_status_icon.sql` strips the `icon` key from every
-- status declared in `metis.projects.statuses`. Sister to
-- `migration_baselines_sqlite/20260607000000__pre_drop_status_icon.sql`
-- (the SQLite baseline).
--
-- Scope: per [[i-jazguvll]], the drop_status_icon migration needs
-- migration-framework coverage on top of the (already-seeded) `j-defaul`
-- row. The seed migration's predecessor wrote `"icon": "..."` into each
-- of the 5 default-project statuses; the drop_status_icon migration's
-- `jsonb_agg(elem - 'icon')` over `jsonb_array_elements(statuses)` must
-- strip the key from *every* row's statuses, not just `j-defaul`.
--
-- This baseline pre-seeds a non-default custom project row whose
-- statuses also carry the legacy `"icon": "<value>"` shape, so the
-- migration's array-rewrite has more than one row to act on and the
-- subsequent `jsonb_agg` exercises the multi-status, multi-row path.

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
    'j-iconfix',
    1,
    'iconfix',
    'Icon Fixture',
    'todo',
    -- Three-status array with `icon` keys interleaved among other fields
    -- to mimic the predecessor seed JSON exactly. The drop migration's
    -- `elem - 'icon'` must leave every other key untouched.
    '[
        {"key":"todo","label":"Todo","icon":"circle","color":"#abcdef","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"doing","label":"Doing","icon":"circle-half","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},
        {"key":"done","label":"Done","icon":"check","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}
    ]'::jsonb,
    'jayantk',
    '/projects/iconfix/prompt.md'
);
