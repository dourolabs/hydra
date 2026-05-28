-- PR-2 of the sessions/worker_run interface redesign
-- (`/designs/sessions-worker-run-interface.md`).
--
-- `SessionMode::Interactive.conversation_resume_from` is superseded by
-- `Session.resumed_from`. The legacy `tasks_v2.conversation_resume_from`
-- column was dropped back in `20260525000000_drop_legacy_session_columns`,
-- but the field still lingers inside the `mode` JSONB column for rows
-- backfilled by `20260523020000_add_session_shape_columns` and for any
-- writes performed since (`spawn_conversation_sessions::stamp_resume_index`).
-- PR-3 drops the Rust field from the API/domain enum; this migration
-- scrubs the stored JSON so PR-3 can ship without a fresh data backfill.
--
-- Strictly additive at the schema level — no DDL.

UPDATE metis.tasks_v2
SET mode = mode - 'conversation_resume_from'
WHERE mode IS NOT NULL
  AND mode ? 'conversation_resume_from';
