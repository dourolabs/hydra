CREATE TABLE IF NOT EXISTS session_events (
    rowid_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    event_data TEXT NOT NULL,                  -- JSON
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (session_id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_session_events_session_id
    ON session_events(session_id);

CREATE TABLE IF NOT EXISTS session_state (
    session_id TEXT NOT NULL PRIMARY KEY,
    data BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
