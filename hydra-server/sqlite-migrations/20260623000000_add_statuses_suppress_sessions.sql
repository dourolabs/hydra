-- Sister to the Postgres `20260623000000_add_statuses_suppress_sessions.sql`.
-- See that file for design notes; SQLite uses `BOOLEAN` (alias for INTEGER)
-- with `DEFAULT FALSE` so existing rows backfill to 0.

ALTER TABLE statuses ADD COLUMN suppress_sessions BOOLEAN NOT NULL DEFAULT FALSE;
