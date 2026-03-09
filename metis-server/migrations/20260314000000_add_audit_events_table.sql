-- Add audit_events table for enterprise audit logging.

CREATE TABLE IF NOT EXISTS metis.audit_events (
    id              TEXT        NOT NULL,
    timestamp       TIMESTAMPTZ NOT NULL,
    actor_id        TEXT        NOT NULL,
    action          TEXT        NOT NULL,
    resource_type   TEXT        NOT NULL,
    resource_id     TEXT        NOT NULL,
    metadata        JSONB,
    PRIMARY KEY (id)
);

-- Index for cursor-based pagination (timestamp DESC, id DESC).
CREATE INDEX IF NOT EXISTS audit_events_pagination_idx
    ON metis.audit_events (timestamp DESC, id DESC);

-- Indexes for common filter queries.
CREATE INDEX IF NOT EXISTS audit_events_actor_id_idx
    ON metis.audit_events (actor_id);
CREATE INDEX IF NOT EXISTS audit_events_resource_type_idx
    ON metis.audit_events (resource_type);
CREATE INDEX IF NOT EXISTS audit_events_resource_id_idx
    ON metis.audit_events (resource_id);
