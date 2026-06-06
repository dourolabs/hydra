-- Nullable column on `metis.conversations_v2` recording the issue id (if
-- any) that spawned the conversation. Defaults NULL for legacy rows.
ALTER TABLE metis.conversations_v2 ADD COLUMN spawned_from TEXT DEFAULT NULL;

CREATE INDEX IF NOT EXISTS idx_conversations_v2_spawned_from
    ON metis.conversations_v2 (spawned_from)
    WHERE spawned_from IS NOT NULL AND is_latest = TRUE;
