-- Add `session_settings_json TEXT NULL` to `metis.statuses`. Stores a
-- serialized `SessionSettings` blob (issue-wire shape) that the spawn
-- dispatchers merge with the issue-level settings — issue wins over
-- status, status wins over global default. A NULL column maps to
-- `SessionSettings::default()` on read.

ALTER TABLE metis.statuses ADD COLUMN session_settings_json TEXT NULL;
