-- Add `session_settings_json TEXT NULL` to `metis.projects`. Stores a
-- serialized `SessionSettings` blob (issue-wire shape) that the spawn
-- dispatchers merge below the issue-level and status-level settings —
-- issue beats status, status beats project, project beats global default.
-- A NULL column maps to `SessionSettings::default()` on read.

ALTER TABLE metis.projects ADD COLUMN session_settings_json TEXT NULL;
