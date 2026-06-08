-- baseline-version: 20260607000000
-- SQLite pre-drop-status-icon baseline. INSERTs are valid against the
-- schema state at sqlite migration
-- `20260607000000_seed_default_project.sql`, immediately before
-- `20260608000000_drop_status_icon.sql` strips the `icon` key from every
-- status declared in `projects.statuses`. Sister to
-- `migration_baselines/20260607000000__pre_drop_status_icon.sql` (the
-- Postgres baseline) — kept independent so postgres-only fixture
-- changes don't ripple here.
--
-- Scope: per [[i-jazguvll]], the drop_status_icon migration needs
-- migration-framework coverage on top of the (already-seeded) `j-defaul`
-- row. The seed migration's predecessor wrote `"icon": "..."` into each
-- of the 5 default-project statuses; the drop_status_icon migration's
-- `json_group_array(json_remove(value, '$.icon'))` over
-- `json_each(statuses)` must strip the key from *every* row's statuses,
-- not just `j-defaul`.
--
-- This baseline pre-seeds a non-default custom project row whose
-- statuses also carry the legacy `"icon": "<value>"` shape, so the
-- migration's array-rewrite has more than one row to act on and the
-- subsequent `json_each` exercises the multi-status, multi-row path.
--
-- SQLite differences vs. the postgres baseline:
--   * Boolean column (`deleted`, `is_latest`) values are INTEGERs (0/1)
--     and have no trigger to backfill `is_latest`; we set it
--     explicitly.
--   * Statuses are stored as JSON TEXT, not `jsonb`.

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
    'j-iconfix',
    1,
    'iconfix',
    'Icon Fixture',
    'todo',
    -- Three-status array with `icon` keys interleaved among other fields
    -- to mimic the predecessor seed JSON exactly. The drop migration's
    -- `json_remove(value, '$.icon')` must leave every other key
    -- untouched.
    '[{"key":"todo","label":"Todo","icon":"circle","color":"#abcdef","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"doing","label":"Doing","icon":"circle-half","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false},{"key":"done","label":"Done","icon":"check","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false}]',
    'jayantk',
    0,
    NULL,
    '/projects/iconfix/prompt.md',
    1
);
