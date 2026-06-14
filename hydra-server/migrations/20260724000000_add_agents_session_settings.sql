-- Add `session_settings_json TEXT NULL` to `metis.agents`. Stores a
-- serialized `SessionSettings` blob (issue-wire shape) that the spawn
-- dispatcher merges below the per-status / per-project / per-issue
-- layers — chat / PM agents declare small machine defaults here without
-- having to set them on every issue. A NULL column maps to
-- `SessionSettings::default()` on read.

ALTER TABLE metis.agents ADD COLUMN session_settings_json TEXT NULL;
