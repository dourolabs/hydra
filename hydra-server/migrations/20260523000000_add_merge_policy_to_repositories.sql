-- Add merge_policy JSONB column to repositories_v2 table.
-- Mirrors the SQLite merge_policy column added in PR-3 of the
-- merge-time-constraints rollout. Stores per-repo merge policy
-- (reviewer groups + merger rule). Nullable; existing rows keep NULL.
ALTER TABLE metis.repositories_v2
    ADD COLUMN IF NOT EXISTS merge_policy JSONB DEFAULT NULL;
