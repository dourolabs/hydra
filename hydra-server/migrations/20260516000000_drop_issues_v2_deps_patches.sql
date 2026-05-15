-- Drop the legacy dependencies and patches JSON columns from issues_v2.
-- These columns are no longer written or read by Rust code; object_relationships
-- is now the source of truth (backfilled by 20260312000000_add_object_relationships_table.sql).
ALTER TABLE metis.issues_v2 DROP COLUMN IF EXISTS dependencies;
ALTER TABLE metis.issues_v2 DROP COLUMN IF EXISTS patches;
