-- Add index on (created_at DESC, id DESC) to support efficient pagination
-- of list_issues queries without requiring a full-table DISTINCT ON scan.
CREATE INDEX IF NOT EXISTS issues_v2_created_at_id_idx
    ON metis.issues_v2 (created_at DESC, id DESC);
