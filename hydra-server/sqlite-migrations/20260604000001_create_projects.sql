-- Projects table (versioned, following the issues/triggers is_latest pattern)
-- and `issues_v2.project_id` column for the per-project configurable issue
-- statuses design (`/designs/per-project-issue-statuses.md` §4 "Storage").
--
-- PR 2/6: store-only; no consumer reads or writes the new column yet —
-- existing issues stay NULL and resolve through `DefaultProject`.

CREATE TABLE IF NOT EXISTS projects (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    key TEXT NOT NULL,
    name TEXT NOT NULL,
    default_status_key TEXT NOT NULL,
    statuses TEXT NOT NULL,
    creator TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS projects_latest_idx ON projects (id, version_number DESC);
CREATE INDEX IF NOT EXISTS projects_creator_idx ON projects (creator) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS projects_is_latest_idx ON projects (id) WHERE is_latest = 1;

-- Enforce uniqueness of `ProjectKey` across live projects. Mirrors the
-- pattern from `documents_v2_path_unique_active_idx`: only the latest,
-- non-deleted row participates, so soft-deleted projects do not prevent
-- a new project from reusing the same key.
CREATE UNIQUE INDEX IF NOT EXISTS projects_key_unique_active_idx
    ON projects (key) WHERE is_latest = 1 AND deleted = 0;

ALTER TABLE issues_v2 ADD COLUMN project_id TEXT;

CREATE INDEX IF NOT EXISTS issues_v2_project_id_idx ON issues_v2 (project_id);
