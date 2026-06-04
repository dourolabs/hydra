-- Pagination indexes on (created_at DESC, id DESC) for list-* queries.
-- Mirrors postgres migrations 20260315000000 and 20260317000000.
CREATE INDEX IF NOT EXISTS issues_v2_created_at_id_idx
    ON issues_v2 (created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS patches_v2_created_at_id_idx
    ON patches_v2 (created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS tasks_v2_created_at_id_idx
    ON tasks_v2 (created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS documents_v2_created_at_id_idx
    ON documents_v2 (created_at DESC, id DESC);
