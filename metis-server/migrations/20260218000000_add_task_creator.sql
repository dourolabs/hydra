-- Add creator column to tasks_v2 so task.creator is persisted and round-trips.
-- Creator is the username of who created the task (nullable).
ALTER TABLE metis.tasks_v2 ADD COLUMN IF NOT EXISTS creator TEXT DEFAULT NULL;
