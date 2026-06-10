-- Rename the `on_enter.kill_sessions` JSONB key to `on_enter.teardown_work`
-- on `metis.statuses`. Mirrors the Rust-side rename of
-- `StatusOnEnter.kill_sessions` → `StatusOnEnter.teardown_work` and the
-- companion automation rename. The Rust struct retains
-- `#[serde(alias = "kill_sessions")]` so historical YAML configs continue
-- to parse; this migration brings stored DB rows up to date so the
-- serialized form on the wire is the new key.
--
-- Strategy: copy `on_enter->'kill_sessions'` into a new `teardown_work`
-- key, then strip the old key. `jsonb_set` with `create_missing := true`
-- inserts `teardown_work`; `on_enter - 'kill_sessions'` drops the old
-- key while preserving the other keys.
--
-- Idempotent: once the rewrite has run, no row matches the WHERE clause
-- so re-running this body is a no-op.

UPDATE metis.statuses
SET on_enter = jsonb_set(
    on_enter - 'kill_sessions',
    '{teardown_work}',
    on_enter->'kill_sessions',
    true
)
WHERE on_enter ? 'kill_sessions';
