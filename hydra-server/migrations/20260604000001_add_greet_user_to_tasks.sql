-- PR-2 of the sessions/worker_run interface redesign
-- (`/designs/sessions-worker-run-interface.md`).
--
-- Add the new `greet_user` column on `metis.tasks_v2` to back the
-- forthcoming `SessionMode::Interactive.greet_user: bool` field
-- (added to the Rust API/domain enum in PR-3). The column is
-- denormalized from the JSON shape — consistent with how
-- `tasks_v2.conversation_id` is denormalized from
-- `mode.Interactive.conversation_id` (kept per the §6 step-16 note in
-- `20260525000000_drop_legacy_session_columns`).
--
-- All existing rows (both interactive and headless) get the
-- `NOT NULL DEFAULT FALSE` value automatically — `greet_user=false`
-- matches the existing implicit behavior where the server waits for
-- the user's first message before invoking the model.

ALTER TABLE metis.tasks_v2
    ADD COLUMN greet_user BOOLEAN NOT NULL DEFAULT FALSE;
