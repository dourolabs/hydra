-- Add is_latest column to documents_v2 table to match PostgreSQL.
-- This enables efficient "latest version" lookups without MAX(version_number)
-- subqueries and is a prerequisite for partial unique indexes on path.

-- 1. Add the column
ALTER TABLE documents_v2 ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 0;

-- 2. Backfill: set is_latest = 1 for the row with MAX(version_number) per document id
UPDATE documents_v2
SET is_latest = 1
WHERE rowid IN (
    SELECT d.rowid
    FROM documents_v2 d
    INNER JOIN (
        SELECT id, MAX(version_number) AS max_vn
        FROM documents_v2
        GROUP BY id
    ) latest ON d.id = latest.id AND d.version_number = latest.max_vn
);

-- 3. Create index for efficient lookups
CREATE INDEX documents_v2_latest_id_idx ON documents_v2 (id) WHERE is_latest = 1;
CREATE INDEX documents_v2_latest_pagination_idx ON documents_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
