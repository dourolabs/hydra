-- Add secrets (JSONB array) to tasks_v2 so Task.secrets persists and round-trips.
ALTER TABLE hydra.tasks_v2 ADD COLUMN IF NOT EXISTS secrets JSONB DEFAULT NULL;
