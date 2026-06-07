-- Seed the "default" project as a real DB row and backfill any
-- pre-existing `issues_v2.project_id IS NULL` rows so the consumer side
-- can drop its in-process `default_project()` fallback (see issue
-- [[i-dqzrijzy]]).
--
-- The seed is byte-equivalent to the previous in-memory singleton
-- (`hydra-server/src/domain/projects.rs::default_project_seed`):
--   * key                = "default"
--   * name               = "Default"
--   * default_status_key = "open"
--   * 5 statuses (open, in-progress, closed, dropped, failed) with the
--     same icon / color / flag / prompt_path values as the Rust seed.
--   * prompt_path        = "/projects/default/prompt.md"
--
-- The migration is idempotent (INSERT OR IGNORE). The seeded
-- `ProjectId` must stay byte-identical to
-- `domain::projects::DEFAULT_PROJECT_ID_STR` ("j-defaul").
INSERT OR IGNORE INTO projects (
    id,
    version_number,
    key,
    name,
    default_status_key,
    statuses,
    creator,
    deleted,
    actor,
    is_latest,
    prompt_path
) VALUES (
    'j-defaul',
    1,
    'default',
    'Default',
    'open',
    '[{"key":"open","label":"Open","icon":"circle","color":"#3498db","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false,"prompt_path":"/projects/default/statuses/open.md"},{"key":"in-progress","label":"In progress","icon":"circle-dot","color":"#f1c40f","unblocks_parents":false,"unblocks_dependents":false,"cascades_to_children":false,"prompt_path":"/projects/default/statuses/in-progress.md"},{"key":"closed","label":"Closed","icon":"check-circle","color":"#2ecc71","unblocks_parents":true,"unblocks_dependents":true,"cascades_to_children":false},{"key":"dropped","label":"Dropped","icon":"x-circle","color":"#795548","unblocks_parents":true,"unblocks_dependents":false,"cascades_to_children":true},{"key":"failed","label":"Failed","icon":"alert-circle","color":"#e74c3c","unblocks_parents":true,"unblocks_dependents":false,"cascades_to_children":true}]',
    'system',
    0,
    NULL,
    1,
    '/projects/default/prompt.md'
);

-- Backfill: every legacy issues_v2 row (any version) with NULL project_id
-- now points at the seeded default project.
UPDATE issues_v2 SET project_id = 'j-defaul' WHERE project_id IS NULL;
