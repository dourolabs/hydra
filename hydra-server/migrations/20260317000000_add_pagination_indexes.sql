-- Add indexes on (created_at DESC, id DESC) to support efficient pagination
-- of list queries without requiring a full-table DISTINCT ON scan.
CREATE INDEX IF NOT EXISTS patches_v2_created_at_id_idx
    ON metis.patches_v2 (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS tasks_v2_created_at_id_idx
    ON metis.tasks_v2 (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS documents_v2_created_at_id_idx
    ON metis.documents_v2 (created_at DESC, id DESC);
