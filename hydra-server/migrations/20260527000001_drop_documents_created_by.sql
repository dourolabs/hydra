-- Drop the `created_by` column from the metis.documents_v2 table.
ALTER TABLE metis.documents_v2 DROP COLUMN IF EXISTS created_by;
