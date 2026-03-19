-- Make dependencies and patches columns nullable on issues_v2.
-- These columns are no longer written to; object_relationships is the source of truth.
ALTER TABLE hydra.issues_v2 ALTER COLUMN dependencies DROP NOT NULL;
ALTER TABLE hydra.issues_v2 ALTER COLUMN patches DROP NOT NULL;
