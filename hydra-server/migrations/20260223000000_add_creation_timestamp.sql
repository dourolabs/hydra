-- Add creation_timestamp column to issues_v2, patches_v2, and documents_v2.
-- Backfill existing rows using the created_at value from version_number = 1.

--------------------------------------------------------------------------------
-- hydra.issues_v2
--------------------------------------------------------------------------------
ALTER TABLE hydra.issues_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE hydra.issues_v2 t SET creation_timestamp = (
    SELECT created_at FROM hydra.issues_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE hydra.issues_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE hydra.issues_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();

--------------------------------------------------------------------------------
-- hydra.patches_v2
--------------------------------------------------------------------------------
ALTER TABLE hydra.patches_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE hydra.patches_v2 t SET creation_timestamp = (
    SELECT created_at FROM hydra.patches_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE hydra.patches_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE hydra.patches_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();

--------------------------------------------------------------------------------
-- hydra.documents_v2
--------------------------------------------------------------------------------
ALTER TABLE hydra.documents_v2 ADD COLUMN IF NOT EXISTS creation_timestamp TIMESTAMPTZ;

UPDATE hydra.documents_v2 t SET creation_timestamp = (
    SELECT created_at FROM hydra.documents_v2 WHERE id = t.id AND version_number = 1
) WHERE creation_timestamp IS NULL;

ALTER TABLE hydra.documents_v2 ALTER COLUMN creation_timestamp SET NOT NULL;
ALTER TABLE hydra.documents_v2 ALTER COLUMN creation_timestamp SET DEFAULT NOW();
