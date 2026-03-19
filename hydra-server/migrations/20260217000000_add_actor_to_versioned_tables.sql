-- Add actor JSONB column to all versioned tables (v1 and v2).
-- This column stores the ActorRef of who performed each mutation.
-- NULL for historical rows that predate actor tracking.

-- v1 tables (JSONB payload-based)
ALTER TABLE hydra.issues ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.patches ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.tasks ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.users ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.actors ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.repositories ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.documents ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;

-- v2 tables (column-based)
ALTER TABLE hydra.issues_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.patches_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.tasks_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.users_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.actors_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.repositories_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
ALTER TABLE hydra.documents_v2 ADD COLUMN IF NOT EXISTS actor JSONB DEFAULT NULL;
