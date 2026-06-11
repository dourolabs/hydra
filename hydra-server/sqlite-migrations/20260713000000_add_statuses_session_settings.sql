-- Sister to the Postgres `20260712000000_add_statuses_session_settings.sql`.
-- Adds `session_settings_json TEXT NULL` to `statuses` for the per-status
-- `SessionSettings` override layer. When NULL (the default for backfilled
-- rows), the read path materializes `SessionSettings::default()` — same as
-- not specifying an override.

ALTER TABLE statuses ADD COLUMN session_settings_json TEXT NULL;
