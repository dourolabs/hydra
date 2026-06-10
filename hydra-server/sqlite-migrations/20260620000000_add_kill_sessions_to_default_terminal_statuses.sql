-- Sister to the Postgres
-- `20260620000000_add_kill_sessions_to_default_terminal_statuses.sql`.
-- See that file for design notes.
--
-- SQLite stores the `on_enter` column as TEXT (JSON1). `json_set`
-- with a JSON-typed third argument (via `json('true')`) creates the
-- `kill_sessions` key if it's missing and overwrites it if present,
-- while preserving any pre-existing keys on the object. The SQLite
-- statuses table has no `is_latest` column — the row is the row.

UPDATE statuses
SET on_enter = json_set(
    COALESCE(on_enter, '{}'),
    '$.kill_sessions',
    json('true')
)
WHERE project_id = 'j-defaul'
  AND key IN ('closed', 'dropped', 'failed');
