-- Seed `on_enter.clear_assignee = true` on the three terminal statuses
-- of the default project (`closed`, `dropped`, `failed`) so the
-- `apply_status_on_enter` automation unsets each issue's assignee on
-- transition into that status. Mirrors the change to
-- `hydra-server/src/domain/projects.rs::default_project_seed` so the
-- SQL-backed and Memory-backed stores stay in lockstep.
--
-- `jsonb_set` with the `create_missing := true` final argument
-- preserves any pre-existing `on_enter` keys (`assign_to`, `attach_form`)
-- on the row; `COALESCE(on_enter, '{}'::jsonb)` handles the today-case
-- where the column is NULL because no `on_enter` block was seeded.
-- Idempotent: re-running the body simply re-sets the key to `true`.

UPDATE metis.statuses
SET on_enter = jsonb_set(
    COALESCE(on_enter, '{}'::jsonb),
    '{clear_assignee}',
    'true'::jsonb,
    true
)
WHERE project_id = 'j-defaul'
  AND key IN ('closed', 'dropped', 'failed');
