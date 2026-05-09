-- Create conversations and conversation_events tables for the v2 store.

--------------------------------------------------------------------------------
-- metis.conversations_v2 — metadata (versioned, same pattern as issues_v2)
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.conversations_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    title TEXT,
    agent_name TEXT,
    active_session_id TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    creator TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    actor JSONB,
    is_latest BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

-- Trigger to auto-update updated_at on row update
DROP TRIGGER IF EXISTS set_timestamp_conversations_v2 ON metis.conversations_v2;
CREATE TRIGGER set_timestamp_conversations_v2
BEFORE UPDATE ON metis.conversations_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

-- Trigger to maintain is_latest flag on insert
CREATE TRIGGER trg_maintain_latest_conversations_v2
    BEFORE INSERT ON metis.conversations_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

-- Indexes
CREATE INDEX conversations_v2_creator_idx
    ON metis.conversations_v2 (creator) WHERE is_latest = true;

CREATE INDEX conversations_v2_latest_pagination_idx
    ON metis.conversations_v2 (created_at DESC, id DESC) WHERE is_latest = true;

CREATE INDEX conversations_v2_latest_id_idx
    ON metis.conversations_v2 (id) WHERE is_latest = true;

--------------------------------------------------------------------------------
-- metis.conversation_events_v2 — event log (append-only)
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.conversation_events_v2 (
    id BIGSERIAL NOT NULL,
    conversation_id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    actor JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id)
);

CREATE UNIQUE INDEX conversation_events_v2_conv_version_idx
    ON metis.conversation_events_v2 (conversation_id, version_number);

CREATE INDEX conversation_events_v2_conversation_id_idx
    ON metis.conversation_events_v2 (conversation_id);

--------------------------------------------------------------------------------
-- metis.conversation_session_state — binary blob (upsert)
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.conversation_session_state (
    conversation_id TEXT NOT NULL PRIMARY KEY,
    data BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
