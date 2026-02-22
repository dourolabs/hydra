-- Add updated_at DESC indexes to support the global activity feed UNION query.
-- These indexes allow each sub-query in the UNION to efficiently find the most
-- recent version rows ordered by updated_at.

CREATE INDEX IF NOT EXISTS issues_v2_updated_at_idx
    ON metis.issues_v2 (updated_at DESC, id DESC, version_number DESC);

CREATE INDEX IF NOT EXISTS patches_v2_updated_at_idx
    ON metis.patches_v2 (updated_at DESC, id DESC, version_number DESC);

CREATE INDEX IF NOT EXISTS tasks_v2_updated_at_idx
    ON metis.tasks_v2 (updated_at DESC, id DESC, version_number DESC);

CREATE INDEX IF NOT EXISTS documents_v2_updated_at_idx
    ON metis.documents_v2 (updated_at DESC, id DESC, version_number DESC);
