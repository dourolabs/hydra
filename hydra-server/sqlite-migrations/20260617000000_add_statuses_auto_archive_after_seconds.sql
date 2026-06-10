-- Sister to the Postgres `20260617000000_add_statuses_auto_archive_after_seconds.sql`.
-- See that file for design notes; SQLite uses `INTEGER` for the BIGINT
-- column (sqlx maps it bidirectionally with i64).

ALTER TABLE statuses ADD COLUMN auto_archive_after_seconds INTEGER;
