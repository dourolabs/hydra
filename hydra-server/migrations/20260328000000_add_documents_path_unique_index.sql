-- Add a partial unique index on document path to prevent duplicate active paths.
-- This closes the TOCTOU race condition in the create_document flow.
-- Only the latest, non-deleted rows with a non-NULL path are constrained.
CREATE UNIQUE INDEX documents_v2_path_unique_active_idx
    ON metis.documents_v2 (path)
    WHERE is_latest = true AND deleted = false AND path IS NOT NULL;
