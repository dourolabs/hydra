-- Drop the `created_by` column from `patches_v2`. The
-- `RunningJobValidationRestriction` no longer reads it (it now derives the
-- session under validation from the operation actor), and all surface
-- consumers (CLI changelog, web UI agent/human heuristic, mock server)
-- have been migrated to drop the field.
ALTER TABLE patches_v2 DROP COLUMN created_by;
