-- Tighten the new session-shape columns on metis.tasks_v2 to NOT NULL.
--
-- These columns were added (nullable) and backfilled in
-- 20260523020000_add_session_shape_columns; PR-5 / Phase E step 16
-- (20260525000000_drop_legacy_session_columns) made them the sole source of
-- session shape and the in-Rust `TaskRow` now treats them as non-optional
-- (e.g. `mount_spec: serde_json::Value`, not `Option<_>`). This migration
-- aligns the schema with that in-Rust assumption.
--
-- `resumed_from` stays nullable: only resumed sessions reference a predecessor.

ALTER TABLE metis.tasks_v2 ALTER COLUMN mount_spec   SET NOT NULL;
ALTER TABLE metis.tasks_v2 ALTER COLUMN agent_config SET NOT NULL;
ALTER TABLE metis.tasks_v2 ALTER COLUMN mode         SET NOT NULL;
