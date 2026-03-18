-- Add actor JSONB column to all versioned tables (v1 and v2).
-- This column stores the ActorRef of who performed each mutation.
-- NULL for historical rows that predate actor tracking.

-- v1 tables (JSONB payload-based)
ALTER TABLE metis.issues ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.patches ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.tasks ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.users ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.actors ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.repositories ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.documents ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;

-- v2 tables (column-based)
ALTER TABLE metis.issues_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.tasks_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.users_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.actors_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.repositories_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE metis.documents_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
