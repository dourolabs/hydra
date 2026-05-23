-- Phase 3 PR-2 of the merge-time-constraints rollout: drop the legacy
-- `patch_workflow` column. Phase 3 PR-1 synthesised a `merge_policy` from
-- every existing `patch_workflow` before this migration runs, so dropping
-- the column does not erase any active gating configuration.
ALTER TABLE metis.repositories_v2
    DROP COLUMN IF EXISTS patch_workflow;
