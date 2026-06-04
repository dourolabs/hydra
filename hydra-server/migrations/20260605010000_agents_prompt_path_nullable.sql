-- Drop the NOT NULL constraint on metis.agents.prompt_path so the column
-- can carry `NULL` for "no path", instead of the legacy empty-string
-- sentinel. Backfill any existing empty-string rows to NULL so the
-- domain type matches storage exactly.
ALTER TABLE metis.agents ALTER COLUMN prompt_path DROP NOT NULL;
UPDATE metis.agents SET prompt_path = NULL WHERE prompt_path = '';
