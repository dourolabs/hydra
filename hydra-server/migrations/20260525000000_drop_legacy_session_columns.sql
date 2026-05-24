-- Phase E step 16 (sessions-orthogonality redesign): drop the legacy
-- `tasks_v2` columns now subsumed by the new `mode`, `agent_config`,
-- `mount_spec`, and `resumed_from` columns (added and backfilled in
-- 20260523020000_add_session_shape_columns).
--
-- Explicitly NOT dropped: `tasks_v2.conversation_id`. Per design §6
-- step 16 it is retained as the §3.4.1 single-query lookup index — it
-- is denormalized from `mode.Interactive.conversation_id` at insert
-- time and never edited independently.

ALTER TABLE metis.tasks_v2 DROP COLUMN context;
ALTER TABLE metis.tasks_v2 DROP COLUMN prompt;
ALTER TABLE metis.tasks_v2 DROP COLUMN interactive;
ALTER TABLE metis.tasks_v2 DROP COLUMN conversation_resume_from;
ALTER TABLE metis.tasks_v2 DROP COLUMN model;
ALTER TABLE metis.tasks_v2 DROP COLUMN mcp_config;
