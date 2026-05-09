-- Conversations metadata table (versioned, following is_latest pattern)
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT,
    agent_name TEXT,
    active_session_id TEXT,
    session_state BLOB,
    status TEXT NOT NULL DEFAULT 'active',
    creator TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    is_latest INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_conversations_creator ON conversations(creator);
CREATE INDEX IF NOT EXISTS idx_conversations_updated_at ON conversations(updated_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS idx_conversations_is_latest ON conversations(id) WHERE is_latest = 1;

-- Conversation events table (versioned per conversation)
CREATE TABLE IF NOT EXISTS conversation_events (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    event_data TEXT NOT NULL,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_conversation_events_id ON conversation_events(id);
