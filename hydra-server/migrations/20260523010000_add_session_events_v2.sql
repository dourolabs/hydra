-- Create session_events_v2 and session_state_v2 tables for the v2 store.
-- Phase B step 4 of designs/sessions-orthogonality-redesign.md §3.3.

--------------------------------------------------------------------------------
-- metis.session_events_v2 — append-only session event log
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.session_events_v2 (
    id BIGSERIAL NOT NULL,
    session_id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    actor JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id)
);

CREATE UNIQUE INDEX session_events_v2_session_version_idx
    ON metis.session_events_v2 (session_id, version_number);
CREATE INDEX session_events_v2_session_id_idx
    ON metis.session_events_v2 (session_id);

--------------------------------------------------------------------------------
-- metis.session_state_v2 — binary blob (upsert)
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.session_state_v2 (
    session_id TEXT NOT NULL PRIMARY KEY,
    data BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
