-- Drop the now-unused `default_status_key` column from `projects`.
-- Frontend pre-fill and board-highlight readers were the only behavior
-- anchored on this column; both have been removed alongside the column
-- drop.
--
-- SQLite ≥ 3.35 supports `ALTER TABLE ... DROP COLUMN` directly, but
-- that form is not idempotent (re-running it errors when the column
-- has already been dropped). The migration-framework test exercises
-- the body twice, so use the table-rebuild dance with
-- `CREATE TABLE IF NOT EXISTS` / `INSERT OR IGNORE` / `DROP TABLE` /
-- `ALTER TABLE RENAME`: a re-run rebuilds an empty `projects_new`,
-- copies the now-already-default-status-key-free rows, and swaps the
-- table back into place, ending in the same shape.
--
-- Column names are enumerated explicitly (NOT `SELECT *`) so a future
-- column add elsewhere doesn't accidentally lose data here.
CREATE TABLE IF NOT EXISTS projects_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    key TEXT NOT NULL,
    name TEXT NOT NULL,
    statuses TEXT NOT NULL,
    creator TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    prompt_path TEXT DEFAULT NULL,
    priority REAL NOT NULL DEFAULT 0.0,
    PRIMARY KEY (id, version_number)
);

INSERT OR IGNORE INTO projects_new (
    id, version_number, key, name, statuses, creator, deleted, actor,
    is_latest, created_at, updated_at, prompt_path, priority
)
SELECT
    id, version_number, key, name, statuses, creator, deleted, actor,
    is_latest, created_at, updated_at, prompt_path, priority
FROM projects;

DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;

CREATE INDEX IF NOT EXISTS projects_latest_idx ON projects (id, version_number DESC);
CREATE INDEX IF NOT EXISTS projects_creator_idx ON projects (creator) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS projects_is_latest_idx ON projects (id) WHERE is_latest = 1;
CREATE UNIQUE INDEX IF NOT EXISTS projects_key_unique_active_idx
    ON projects (key) WHERE is_latest = 1 AND deleted = 0;
