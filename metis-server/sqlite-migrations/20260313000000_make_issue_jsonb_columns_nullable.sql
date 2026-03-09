-- Phase 3: dependencies and patches columns are no longer written.
-- The object_relationships table is the sole source of truth.
-- SQLite does not support ALTER COLUMN, but the columns already have
-- DEFAULT '[]' so no schema change is needed; new rows will simply
-- receive NULL from the application layer.

-- Allow NULL in dependencies and patches by recreating the table.
-- SQLite requires a full table rebuild to change column constraints.
CREATE TABLE issues_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    issue_type TEXT NOT NULL,
    description TEXT NOT NULL,
    creator TEXT NOT NULL,
    progress TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    assignee TEXT,
    job_settings TEXT NOT NULL DEFAULT '{}',
    todo_list TEXT NOT NULL DEFAULT '[]',
    dependencies TEXT DEFAULT NULL,
    patches TEXT DEFAULT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

INSERT INTO issues_v2_new SELECT * FROM issues_v2;
DROP TABLE issues_v2;
ALTER TABLE issues_v2_new RENAME TO issues_v2;

CREATE INDEX IF NOT EXISTS issues_v2_status_idx ON issues_v2 (status);
CREATE INDEX IF NOT EXISTS issues_v2_latest_idx ON issues_v2 (id, version_number DESC);
