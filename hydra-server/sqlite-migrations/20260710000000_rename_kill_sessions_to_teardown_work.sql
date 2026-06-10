-- Sister to the Postgres
-- `20260710000000_rename_kill_sessions_to_teardown_work.sql`.
-- See that file for design notes.
--
-- SQLite stores `on_enter` as TEXT (JSON1) and has no native boolean
-- type — `json_extract` of a JSON boolean returns the integer 0/1, which
-- would round-trip through `json_set` as an integer literal rather than
-- a JSON boolean. The historical seed only ever stored
-- `kill_sessions: true`, so write the literal `json('true')` value into
-- the new `teardown_work` key and drop the old one with `json_remove`.
-- The `WHERE` clause restricts the rewrite to rows that still carry the
-- legacy `kill_sessions` key, so re-running this body on already-
-- migrated rows is a no-op.

UPDATE statuses
SET on_enter = json_remove(
    json_set(on_enter, '$.teardown_work', json('true')),
    '$.kill_sessions'
)
WHERE on_enter IS NOT NULL
  AND json_extract(on_enter, '$.kill_sessions') IS NOT NULL;
