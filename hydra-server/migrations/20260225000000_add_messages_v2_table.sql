-- Add messages_v2 table for the versioned messaging system.
-- Messages use a composite PK (id, version_number) following the same
-- pattern as issues_v2, patches_v2, tasks_v2, and documents_v2.

CREATE TABLE IF NOT EXISTS metis.messages_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    conversation_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    actor JSONB DEFAULT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_messages_v2_conversation ON metis.messages_v2 (conversation_id, id DESC);
CREATE INDEX IF NOT EXISTS idx_messages_v2_latest ON metis.messages_v2 (id, version_number DESC);
CREATE INDEX IF NOT EXISTS idx_messages_v2_sender ON metis.messages_v2 (sender);

DROP TRIGGER IF EXISTS set_timestamp_messages_v2 ON metis.messages_v2;
CREATE TRIGGER set_timestamp_messages_v2
BEFORE UPDATE ON metis.messages_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();
