-- Sister to the Postgres `20260616000000_add_statuses_position.sql`.
-- See that file for design notes; SQLite uses `REAL` for the float
-- column (sqlx maps it bidirectionally with f64).

ALTER TABLE statuses ADD COLUMN position REAL NOT NULL DEFAULT 0;

UPDATE statuses SET position = sequence;
