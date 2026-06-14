-- Sister to the Postgres `20260724000000_add_agents_session_settings.sql`.
-- Adds `session_settings_json TEXT NULL` to `agents` for the per-agent
-- `SessionSettings` default layer. The spawn dispatcher merges this layer
-- below status / project / issue so chat / PM agents can declare small
-- defaults without per-issue overrides. When NULL (the default for
-- backfilled rows), the read path materializes `SessionSettings::default()`.

ALTER TABLE agents ADD COLUMN session_settings_json TEXT NULL;
