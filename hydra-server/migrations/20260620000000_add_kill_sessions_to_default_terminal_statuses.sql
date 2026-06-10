-- Seed `on_enter.kill_sessions = true` on the three terminal statuses
-- of the default project (`closed`, `dropped`, `failed`) so the
-- `kill_sessions_on_enter` automation tears down any active sessions
-- attached to an issue when it transitions into one of these statuses.
-- Mirrors the change to
-- `hydra-server/src/domain/projects.rs::default_project_seed` so the
-- SQL-backed and Memory-backed stores stay in lockstep.
--
-- `jsonb_set` with the `create_missing := true` final argument
-- preserves any pre-existing `on_enter` keys (`assign_to`,
-- `attach_form`, `clear_assignee`) on the row; `COALESCE(on_enter,
-- '{}'::jsonb)` handles the case where the column is NULL because no
-- `on_enter` block was seeded. Idempotent: re-running the body simply
-- re-sets the key to `true`.

UPDATE metis.statuses
SET on_enter = jsonb_set(
    COALESCE(on_enter, '{}'::jsonb),
    '{kill_sessions}',
    'true'::jsonb,
    true
)
WHERE project_id = 'j-defaul'
  AND key IN ('closed', 'dropped', 'failed');
