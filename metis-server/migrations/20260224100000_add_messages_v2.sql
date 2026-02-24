CREATE TABLE metis.messages_v2 (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,
    actor JSONB NOT NULL
);

CREATE INDEX idx_messages_v2_conversation ON metis.messages_v2 (conversation_id, message_id DESC);
CREATE INDEX idx_messages_v2_sender ON metis.messages_v2 (sender);
