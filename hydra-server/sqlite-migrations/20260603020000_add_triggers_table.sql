-- Triggers table (versioned, following the issues/conversations is_latest pattern).
-- `last_fired_at` is updated in-place by `Store::record_trigger_fire` — see
-- `/designs/triggered-actions.md` §4.4 / §4.6.
CREATE TABLE IF NOT EXISTS triggers (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    enabled INTEGER NOT NULL,
    creator TEXT NOT NULL,
    schedule TEXT NOT NULL,
    actions TEXT NOT NULL,
    last_fired_at TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS triggers_latest_idx ON triggers (id, version_number DESC);
CREATE INDEX IF NOT EXISTS triggers_creator_idx ON triggers (creator) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS triggers_is_latest_idx ON triggers (id) WHERE is_latest = 1;
