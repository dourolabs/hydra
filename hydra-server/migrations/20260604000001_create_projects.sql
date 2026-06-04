-- Projects table (versioned, mirrors the issues/triggers is_latest pattern)
-- and `issues_v2.project_id` column for the per-project configurable issue
-- statuses design (`/designs/per-project-issue-statuses.md` §4 "Storage").
--
-- PR 2/6: store-only; no consumer reads or writes the new column yet —
-- existing issues stay NULL and resolve through `DefaultProject`.
CREATE TABLE IF NOT EXISTS metis.projects (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    key TEXT NOT NULL,
    name TEXT NOT NULL,
    default_status_key TEXT NOT NULL,
    statuses JSONB NOT NULL,
    creator TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    actor JSONB,
    is_latest BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

DROP TRIGGER IF EXISTS set_timestamp_projects ON metis.projects;
CREATE TRIGGER set_timestamp_projects
BEFORE UPDATE ON metis.projects
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

CREATE TRIGGER trg_maintain_latest_projects
    BEFORE INSERT ON metis.projects
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE INDEX projects_creator_idx
    ON metis.projects (creator) WHERE is_latest = true;

CREATE INDEX projects_latest_id_idx
    ON metis.projects (id) WHERE is_latest = true;

ALTER TABLE metis.issues_v2 ADD COLUMN IF NOT EXISTS project_id TEXT;

CREATE INDEX IF NOT EXISTS issues_v2_project_id_idx ON metis.issues_v2 (project_id);
