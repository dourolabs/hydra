-- Drop the unused creation_timestamp column from issues_v2, patches_v2, and documents_v2.
-- Creation time is already derived from version history using MIN(created_at) subqueries.

ALTER TABLE hydra.issues_v2 DROP COLUMN IF EXISTS creation_timestamp;
ALTER TABLE hydra.patches_v2 DROP COLUMN IF EXISTS creation_timestamp;
ALTER TABLE hydra.documents_v2 DROP COLUMN IF EXISTS creation_timestamp;
