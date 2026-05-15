-- Drop redundant JSON columns from issues_v2.
--
-- These columns are no longer written (the INSERT path omits them and they
-- default to '[]') and no read path queries them. Issue dependencies and
-- patches are sourced from the `object_relationships` table, which was
-- backfilled by the 2026-03-12 migration.

ALTER TABLE issues_v2 DROP COLUMN dependencies;
ALTER TABLE issues_v2 DROP COLUMN patches;
