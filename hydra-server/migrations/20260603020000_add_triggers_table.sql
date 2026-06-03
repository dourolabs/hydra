-- Triggers table (versioned, mirrors the issues/conversations is_latest pattern).
-- `last_fired_at` is updated in-place by `Store::record_trigger_fire` —
-- see `/designs/triggered-actions.md` §4.4 / §4.6.
CREATE TABLE IF NOT EXISTS metis.triggers (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    enabled BOOLEAN NOT NULL,
    creator TEXT NOT NULL,
    schedule JSONB NOT NULL,
    actions JSONB NOT NULL,
    last_fired_at TIMESTAMPTZ,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    actor JSONB,
    is_latest BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

DROP TRIGGER IF EXISTS set_timestamp_triggers ON metis.triggers;
CREATE TRIGGER set_timestamp_triggers
BEFORE UPDATE ON metis.triggers
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

CREATE TRIGGER trg_maintain_latest_triggers
    BEFORE INSERT ON metis.triggers
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE INDEX triggers_creator_idx
    ON metis.triggers (creator) WHERE is_latest = true;

CREATE INDEX triggers_latest_id_idx
    ON metis.triggers (id) WHERE is_latest = true;
