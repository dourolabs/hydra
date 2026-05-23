-- Phase D PR-2 follow-up to migration 20260523020000_add_session_shape_columns.
--
-- That migration backfilled `mode` for interactive historical rows with
-- `"idle_timeout_secs": 0` because the value was never persisted per-row.
-- PR-2 then made `SessionMode::Interactive::idle_timeout_secs` an
-- `Option<u64>` where `None` means "apply the server-configured default"
-- and `Some(0)` would suspend a freshly-started session immediately.
--
-- Sentinel `0` values backfilled by the prior migration must therefore be
-- stripped so that they are read as `None` (i.e. "use the configured
-- default") rather than `Some(0)`.
UPDATE tasks_v2
SET mode = json_remove(mode, '$.idle_timeout_secs')
WHERE mode IS NOT NULL
  AND json_extract(mode, '$.idle_timeout_secs') = 0;
