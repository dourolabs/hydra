-- Make the prompt column nullable so that sessions without a prompt
-- (e.g., resumed conversations) store NULL instead of an empty string.
ALTER TABLE metis.tasks_v2 ALTER COLUMN prompt DROP NOT NULL;

-- Convert existing empty-string prompts to NULL for consistency.
UPDATE metis.tasks_v2 SET prompt = NULL WHERE prompt = '';
