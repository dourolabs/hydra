ALTER TABLE conversations ADD COLUMN spawned_from TEXT;

CREATE INDEX IF NOT EXISTS idx_conversations_spawned_from ON conversations(spawned_from) WHERE spawned_from IS NOT NULL AND is_latest = 1;
