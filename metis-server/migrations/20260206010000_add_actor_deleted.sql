-- Add deleted column to actors_v2 table for soft-delete support.
ALTER TABLE metis.actors_v2 ADD COLUMN IF NOT EXISTS deleted BOOLEAN NOT NULL DEFAULT FALSE;
