-- Add branch_name and commit_range columns to patches_v2.
-- Both are nullable for backward compatibility with existing patches.
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS branch_name TEXT;
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS commit_range JSONB;
