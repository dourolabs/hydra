-- Sister to the Postgres `20260712000000_add_statuses_max_simultaneous_sessions.sql`.
-- See that file for design notes; SQLite uses `INTEGER` for the BIGINT
-- column (sqlx maps it bidirectionally with i64).

ALTER TABLE statuses ADD COLUMN max_simultaneous_sessions INTEGER;
