-- Add is_latest column to the 7 remaining versioned tables to match PostgreSQL.
-- documents_v2 already has is_latest (20260328000000_add_is_latest_to_documents.sql).

-- 1. Add the column to all 7 tables
ALTER TABLE repositories_v2 ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE actors_v2       ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users_v2        ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE issues_v2       ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE patches_v2      ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks_v2        ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages_v2     ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;

-- 2. Backfill: set is_latest = 1 for the row with MAX(version_number) per id
UPDATE repositories_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT r.rowid FROM repositories_v2 r
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM repositories_v2 GROUP BY id) latest
    ON r.id = latest.id AND r.version_number = latest.max_vn
);

UPDATE actors_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT a.rowid FROM actors_v2 a
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM actors_v2 GROUP BY id) latest
    ON a.id = latest.id AND a.version_number = latest.max_vn
);

UPDATE users_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT u.rowid FROM users_v2 u
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM users_v2 GROUP BY id) latest
    ON u.id = latest.id AND u.version_number = latest.max_vn
);

UPDATE issues_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT i.rowid FROM issues_v2 i
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM issues_v2 GROUP BY id) latest
    ON i.id = latest.id AND i.version_number = latest.max_vn
);

UPDATE patches_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT p.rowid FROM patches_v2 p
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM patches_v2 GROUP BY id) latest
    ON p.id = latest.id AND p.version_number = latest.max_vn
);

UPDATE tasks_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT t.rowid FROM tasks_v2 t
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM tasks_v2 GROUP BY id) latest
    ON t.id = latest.id AND t.version_number = latest.max_vn
);

UPDATE messages_v2 SET is_latest = 1
WHERE rowid IN (
    SELECT m.rowid FROM messages_v2 m
    INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM messages_v2 GROUP BY id) latest
    ON m.id = latest.id AND m.version_number = latest.max_vn
);

-- 3. Create partial indexes for efficient lookups by id
CREATE INDEX repositories_v2_latest_id_idx ON repositories_v2 (id) WHERE is_latest = 1;
CREATE INDEX actors_v2_latest_id_idx       ON actors_v2       (id) WHERE is_latest = 1;
CREATE INDEX users_v2_latest_id_idx        ON users_v2        (id) WHERE is_latest = 1;
CREATE INDEX issues_v2_latest_id_idx       ON issues_v2       (id) WHERE is_latest = 1;
CREATE INDEX patches_v2_latest_id_idx      ON patches_v2      (id) WHERE is_latest = 1;
CREATE INDEX tasks_v2_latest_id_idx        ON tasks_v2        (id) WHERE is_latest = 1;
CREATE INDEX messages_v2_latest_id_idx     ON messages_v2     (id) WHERE is_latest = 1;

-- 4. Create partial indexes for efficient pagination
CREATE INDEX repositories_v2_latest_pagination_idx ON repositories_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX actors_v2_latest_pagination_idx       ON actors_v2       (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX users_v2_latest_pagination_idx        ON users_v2        (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX issues_v2_latest_pagination_idx       ON issues_v2       (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX patches_v2_latest_pagination_idx      ON patches_v2      (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX tasks_v2_latest_pagination_idx        ON tasks_v2        (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX messages_v2_latest_pagination_idx     ON messages_v2     (created_at DESC, id DESC) WHERE is_latest = 1;
