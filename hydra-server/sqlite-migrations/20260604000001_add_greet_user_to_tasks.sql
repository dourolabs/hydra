-- PR-2 of the sessions/worker_run interface redesign
-- (`/designs/sessions-worker-run-interface.md`).
--
-- SQLite parity for the postgres migration of the same name. SQLite ≥
-- 3.35 supports `ALTER TABLE ... ADD COLUMN ... NOT NULL DEFAULT ...`
-- natively, so we do NOT need the rename-table-rebuild dance.

ALTER TABLE tasks_v2
    ADD COLUMN greet_user BOOLEAN NOT NULL DEFAULT 0;
