-- PR-2 of the sessions/worker_run interface redesign
-- (`/designs/sessions-worker-run-interface.md`).
--
-- Mirror of the postgres migration of the same name. The legacy
-- `tasks_v2.conversation_resume_from` column was already dropped in
-- `20260525000000_drop_legacy_session_columns`; this scrubs the
-- equivalent key out of the `mode` JSON column so PR-3 can drop the
-- Rust field cleanly. Strictly additive at the schema level — no DDL.

UPDATE tasks_v2
SET mode = json_remove(mode, '$.conversation_resume_from')
WHERE mode IS NOT NULL
  AND json_extract(mode, '$.conversation_resume_from') IS NOT NULL;
