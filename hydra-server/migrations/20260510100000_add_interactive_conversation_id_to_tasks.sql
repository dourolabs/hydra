ALTER TABLE metis.tasks_v2 ADD COLUMN interactive BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.tasks_v2 ADD COLUMN conversation_id TEXT;
