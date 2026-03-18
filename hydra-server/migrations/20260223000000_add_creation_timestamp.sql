-- Add creation_timestamp column to issues_v2, patches_v2, and documents_v2.
-- Backfill existing rows using the created_at value from version_number = 1.

--------------------------------------------------------------------------------
-- metis.issues_v2
--------------------------------------------------------------------------------
ALTER TABLE metis.issues_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE metis.issues_v2 t SET creation_timestamp = (
    SELECT created_at FROM metis.issues_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE metis.issues_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE metis.issues_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();

--------------------------------------------------------------------------------
-- metis.patches_v2
--------------------------------------------------------------------------------
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE metis.patches_v2 t SET creation_timestamp = (
    SELECT created_at FROM metis.patches_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE metis.patches_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE metis.patches_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();

--------------------------------------------------------------------------------
-- metis.documents_v2
--------------------------------------------------------------------------------
ALTER TABLE metis.documents_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE metis.documents_v2 t SET creation_timestamp = (
    SELECT created_at FROM metis.documents_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE metis.documents_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE metis.documents_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();
