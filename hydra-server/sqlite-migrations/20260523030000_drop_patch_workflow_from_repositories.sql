-- Phase 3 PR-2 of the merge-time-constraints rollout: remove the legacy
-- `patch_workflow` column now that `merge_policy` (Phase 1) plus the
-- `merge_authorization` restriction (Phase 2) cover all gating. Phase 3 PR-1
-- synthesised a `merge_policy` from every existing `patch_workflow` row before
-- this column is dropped.
ALTER TABLE repositories_v2 DROP COLUMN patch_workflow;
