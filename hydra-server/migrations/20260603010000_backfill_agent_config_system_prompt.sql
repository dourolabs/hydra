-- Append-only follow-up to 20260523020000_add_session_shape_columns.
--
-- PR-1 [[p-mawejdad]] originally attempted to do this work in-place on
-- 20260523020000 (setting `agent_config.system_prompt = prompt` during the
-- initial backfill). That in-place edit was reverted because migrations are
-- append-only once they land on `main` — editing a migration's body after it
-- has been applied to a production database means later environments would
-- get a different cumulative result than databases that already ran the
-- original version. This migration finishes that work as a fresh, appended
-- step.
--
-- The post-PR-1 read path sources the worker prompt from
-- `agent_config.system_prompt` for both modes; historical rows had
-- `agent_config.system_prompt` set to NULL by 20260523020000, with the
-- prompt instead living on `mode.prompt` for headless rows (the legacy
-- `prompt` column itself was dropped by 20260525000000). Copy that prompt
-- onto `agent_config.system_prompt` so the read path stays correct.
--
-- Idempotent: the WHERE clause restricts the update to rows that still have
-- `agent_config->>'system_prompt'` NULL, so re-running is a no-op.

UPDATE metis.tasks_v2
SET agent_config = jsonb_set(
    agent_config,
    '{system_prompt}',
    to_jsonb(mode->>'prompt')
)
WHERE agent_config->>'system_prompt' IS NULL
  AND mode->>'prompt' IS NOT NULL;
