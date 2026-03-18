-- Add creator (username) and base_branch to patches_v2 so Patch.creator and
-- Patch.base_branch persist and round-trip.
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS creator TEXT;
ALTER TABLE metis.patches_v2 ADD COLUMN IF NOT EXISTS base_branch TEXT;
