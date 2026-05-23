-- Phase D step 12 (sessions-orthogonality redesign): add the four new
-- session-shape columns to metis.tasks_v2 and backfill them from the
-- existing legacy columns. Additive only — the legacy columns
-- (context, prompt, model, mcp_config, interactive, conversation_id,
-- conversation_resume_from) are left in place and continue to be the
-- read path. Phase E step 16 will drop them in a later migration.

ALTER TABLE metis.tasks_v2 ADD COLUMN mount_spec   JSONB;
ALTER TABLE metis.tasks_v2 ADD COLUMN agent_config JSONB;
ALTER TABLE metis.tasks_v2 ADD COLUMN mode         JSONB;
ALTER TABLE metis.tasks_v2 ADD COLUMN resumed_from TEXT;

--------------------------------------------------------------------------------
-- mount_spec backfill
--
-- Mirrors the standard 2-item layout (Bundle + Documents) produced by
-- `hydra-server/src/routes/sessions/context.rs::build_mount_spec` for rows
-- with no associated BuildCache. The BuildCache mount item is config-derived
-- (not stored per-row), so it is intentionally omitted from the backfill;
-- runtime spec construction adds it when applicable.
--
-- `bundle` is copied verbatim from the legacy `context` JSONB; the JSON shape
-- of BundleSpec overlaps with Bundle for the variants stored historically
-- (`none`, `git_repository`). ServiceRepository rows retain their existing
-- shape under the `bundle` field — PR-3 (server-side translation rewrite)
-- replaces them with resolved values.
--------------------------------------------------------------------------------
UPDATE metis.tasks_v2
SET mount_spec = jsonb_build_object(
    'working_dir', 'repo',
    'mounts', jsonb_build_array(
        jsonb_build_object(
            'type', 'bundle',
            'target', 'repo',
            'bundle', context,
            'session_id', id
        ),
        jsonb_build_object(
            'type', 'documents',
            'target', 'documents'
        )
    )
)
WHERE mount_spec IS NULL;

--------------------------------------------------------------------------------
-- agent_config backfill
--
-- `system_prompt` is intentionally NULL on historical rows — resolving it
-- from the agent definition is out of scope here (deferred to PR-3).
-- `agent_name` is also NULL on historical rows: the legacy schema does not
-- record which agent was used at session-creation time.
--------------------------------------------------------------------------------
UPDATE metis.tasks_v2
SET agent_config = jsonb_build_object(
    'agent_name',    NULL,
    'model',         model,
    'system_prompt', NULL,
    'mcp_config',    mcp_config
)
WHERE agent_config IS NULL;

--------------------------------------------------------------------------------
-- mode backfill
--
-- Headless if no conversation_id was attached, Interactive otherwise.
-- `idle_timeout_secs` defaults to 0 for historical rows because the value was
-- never persisted per-row (it lives in server config); PR-3 picks up the
-- current config value at runtime.
--------------------------------------------------------------------------------
UPDATE metis.tasks_v2
SET mode = CASE
    WHEN conversation_id IS NULL THEN
        jsonb_build_object('type', 'headless', 'prompt', prompt)
    ELSE
        jsonb_build_object(
            'type', 'interactive',
            'conversation_id', conversation_id,
            'idle_timeout_secs', 0
        )
END
WHERE mode IS NULL;

--------------------------------------------------------------------------------
-- resumed_from backfill
--
-- Walks the existing per-conversation resumption chain (today inferred from
-- `conversation_resume_from` together with create-time ordering) and points
-- each resumed session at its predecessor in the same conversation. This is
-- the Phase C step 10 deferred backfill, now rolled in here.
--
-- The predecessor is defined as the latest-`is_latest` session linked to the
-- same conversation that was created strictly before this one. Rows where
-- the predecessor cannot be identified (e.g. the prior session has since
-- been deleted) are left NULL — PR-3 surfaces these as "predecessor
-- unavailable" at resume time.
--------------------------------------------------------------------------------
UPDATE metis.tasks_v2 t
SET resumed_from = (
    SELECT prev.id
    FROM metis.tasks_v2 prev
    WHERE prev.conversation_id   = t.conversation_id
      AND prev.is_latest         = TRUE
      AND prev.id                <> t.id
      AND prev.creation_time IS NOT NULL
      AND t.creation_time    IS NOT NULL
      AND prev.creation_time     <  t.creation_time
    ORDER BY prev.creation_time DESC
    LIMIT 1
)
WHERE t.conversation_resume_from IS NOT NULL
  AND t.is_latest                = TRUE
  AND t.resumed_from IS NULL;
