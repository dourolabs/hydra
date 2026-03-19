-- Add deleted column to users_v2 table for soft deletion support

ALTER TABLE hydra.users_v2
ADD COLUMN IF NOT EXISTS deleted BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS users_v2_deleted_idx
    ON hydra.users_v2 (id, version_number DESC) WHERE NOT deleted;
