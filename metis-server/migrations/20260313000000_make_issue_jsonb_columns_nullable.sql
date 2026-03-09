-- Phase 3: Make dependencies and patches JSONB columns nullable.
-- These columns are no longer written; the object_relationships table is
-- the sole source of truth.

ALTER TABLE metis.issues_v2 ALTER COLUMN dependencies DROP NOT NULL;
ALTER TABLE metis.issues_v2 ALTER COLUMN dependencies SET DEFAULT NULL;

ALTER TABLE metis.issues_v2 ALTER COLUMN patches DROP NOT NULL;
ALTER TABLE metis.issues_v2 ALTER COLUMN patches SET DEFAULT NULL;
