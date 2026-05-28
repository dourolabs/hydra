-- Phase D step 12 (sessions-orthogonality redesign): add the four new
-- session-shape columns to tasks_v2 and backfill them from the existing
-- legacy columns. Additive only — the legacy columns
-- (context, prompt, model, mcp_config, interactive, conversation_id,
-- conversation_resume_from) are left in place and continue to be the
-- read path. Phase E step 16 will drop them in a later migration.

ALTER TABLE tasks_v2 ADD COLUMN mount_spec   TEXT;
ALTER TABLE tasks_v2 ADD COLUMN agent_config TEXT;
ALTER TABLE tasks_v2 ADD COLUMN mode         TEXT;
ALTER TABLE tasks_v2 ADD COLUMN resumed_from TEXT;

--------------------------------------------------------------------------------
-- mount_spec backfill
--
-- Mirrors the standard 2-item layout (Bundle + Documents) produced by
-- `hydra-server/src/routes/sessions/context.rs::build_mount_spec` for rows
-- with no associated BuildCache. The BuildCache mount item is config-derived
-- (not stored per-row), so it is intentionally omitted here.
--
-- `bundle` is copied verbatim from the legacy `context` JSON; the JSON shape
-- of BundleSpec overlaps with Bundle for the variants stored historically
-- (`none`, `git_repository`). ServiceRepository rows retain their existing
-- shape under the `bundle` field — PR-3 (server-side translation rewrite)
-- replaces them with resolved values.
--------------------------------------------------------------------------------
UPDATE tasks_v2
SET mount_spec = json_object(
    'working_dir', 'repo',
    'mounts', json_array(
        json_object(
            'type', 'bundle',
            'target', 'repo',
            'bundle', json(context),
            'session_id', id
        ),
        json_object(
            'type', 'documents',
            'target', 'documents'
        )
    )
)
WHERE mount_spec IS NULL;

--------------------------------------------------------------------------------
-- agent_config backfill
--
-- `system_prompt` is backfilled from the legacy `prompt` column so the
-- post-PR-1 read path (which sources the worker prompt from
-- `agent_config.system_prompt` for both modes) stays correct for historic
-- rows.
-- `agent_name` is NULL on historical rows: the legacy schema does not
-- record which agent was used at session-creation time.
--------------------------------------------------------------------------------
UPDATE tasks_v2
SET agent_config = json_object(
    'agent_name',    NULL,
    'model',         model,
    'system_prompt', prompt,
    'mcp_config',    CASE WHEN mcp_config IS NULL THEN NULL ELSE json(mcp_config) END
)
WHERE agent_config IS NULL;

--------------------------------------------------------------------------------
-- mode backfill
--
-- Headless if no conversation_id was attached, Interactive otherwise.
-- `idle_timeout_secs` defaults to 0 for historical rows because the value was
-- never persisted per-row (it lives in server config); PR-3 picks up the
-- current config value at runtime.
--
-- Note on divergence from design §6 step 12. The design wording phrases the
-- rule in terms of the legacy `interactive` column: "Headless if `interactive`
-- is None, Interactive otherwise." We key on `conversation_id IS NULL` instead
-- because the new `SessionMode::Interactive { conversation_id, idle_timeout_secs }`
-- variant requires a non-null `conversation_id` (see design §1.3) — there is
-- no way to represent the legacy edge case `interactive=true AND
-- conversation_id IS NULL` in the new shape, so those rows collapse to
-- Headless. The same rule lives in `store/mod.rs::dual_write_mode_json`.
--------------------------------------------------------------------------------
UPDATE tasks_v2
SET mode = CASE
    WHEN conversation_id IS NULL THEN
        json_object('type', 'headless')
    ELSE
        json_object(
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
UPDATE tasks_v2 AS t
SET resumed_from = (
    SELECT prev.id
    FROM tasks_v2 AS prev
    WHERE prev.conversation_id   = t.conversation_id
      AND prev.is_latest         = 1
      AND prev.id                <> t.id
      AND prev.creation_time IS NOT NULL
      AND t.creation_time    IS NOT NULL
      AND prev.creation_time     <  t.creation_time
    ORDER BY prev.creation_time DESC
    LIMIT 1
)
WHERE t.conversation_resume_from IS NOT NULL
  AND t.is_latest                = 1
  AND t.resumed_from IS NULL;
