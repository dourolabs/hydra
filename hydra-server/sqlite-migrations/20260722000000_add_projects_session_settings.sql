-- Sister to the Postgres `20260722000000_add_projects_session_settings.sql`.
-- Adds `session_settings_json TEXT NULL` to `projects` for the per-project
-- `SessionSettings` override layer. When NULL (the default for backfilled
-- rows), the read path materializes `SessionSettings::default()` — same as
-- not specifying an override.

ALTER TABLE projects ADD COLUMN session_settings_json TEXT NULL;
