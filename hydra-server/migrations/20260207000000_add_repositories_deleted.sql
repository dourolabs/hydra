-- Add deleted column to repositories_v2 table for soft deletion
ALTER TABLE hydra.repositories_v2
    ADD COLUMN IF NOT EXISTS deleted BOOLEAN NOT NULL DEFAULT FALSE;
