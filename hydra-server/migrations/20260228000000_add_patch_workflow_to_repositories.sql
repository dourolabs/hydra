-- Add patch_workflow JSONB column to repositories_v2 table.
-- Stores per-repo workflow configuration for patch review and merge.
ALTER TABLE hydra.repositories_v2
    ADD COLUMN IF NOT EXISTS patch_workflow JSONB DEFAULT NULL;
