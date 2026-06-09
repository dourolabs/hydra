-- Tighten `issues_v2.project_id` to NOT NULL.
--
-- The sibling `20260607000000_seed_default_project.sql` migration seeded
-- the `j-defaul` project row and backfilled every legacy NULL
-- `issues_v2.project_id` to it. This migration enforces the invariant at
-- the schema level so the Rust type cutover (Issue.project_id:
-- Option<ProjectId> -> ProjectId, IssueRow.project_id: Option<String> ->
-- String) cannot regress.
--
-- SQLite cannot ALTER COLUMN, so this is the canonical table-rebuild
-- dance: create a new table with the tightened column, copy every row
-- via an explicit named-column INSERT (never SELECT *), drop the old
-- table, rename the new one, and recreate every index that was on
-- `issues_v2`.
--
-- Pre-flight guard: any surviving NULL `project_id` row trips the
-- `project_id TEXT NOT NULL` constraint on `issues_v2_new` below when the
-- column-by-column copy runs, aborting the entire migration with the
-- standard sqlite error
-- (`NOT NULL constraint failed: issues_v2_new.project_id`). The
-- 2026-06-07 backfill should have cleared them; this is
-- belt-and-suspenders. SQLite does not support `RAISE(FAIL, ...)` outside
-- of a trigger body, so we lean on the NOT NULL violation instead.

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
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    form TEXT DEFAULT NULL,
    form_response TEXT DEFAULT NULL,
    feedback TEXT DEFAULT NULL,
    is_latest INTEGER NOT NULL DEFAULT 0,
    assignee_principal TEXT,
    project_id TEXT NOT NULL,
    PRIMARY KEY (id, version_number)
);

-- Copy every column by name. Per the memory rule on SQLite column-
-- reorder migrations, never use `SELECT *` here — explicit names catch
-- schema drift loud.
INSERT INTO issues_v2_new (
    id, version_number, title, issue_type, description, creator, progress,
    status, assignee, job_settings, deleted, actor, created_at, updated_at,
    form, form_response, feedback, is_latest, assignee_principal, project_id
)
SELECT
    id, version_number, title, issue_type, description, creator, progress,
    status, assignee, job_settings, deleted, actor, created_at, updated_at,
    form, form_response, feedback, is_latest, assignee_principal, project_id
FROM issues_v2;

DROP TABLE issues_v2;
ALTER TABLE issues_v2_new RENAME TO issues_v2;

-- Recreate every index that lived on the old table. List verbatim so a
-- future drift between this rebuild and the index set fails the
-- migration-framework test.
CREATE INDEX IF NOT EXISTS issues_v2_status_idx ON issues_v2 (status);
CREATE INDEX IF NOT EXISTS issues_v2_latest_idx ON issues_v2 (id, version_number DESC);
CREATE INDEX IF NOT EXISTS issues_v2_latest_id_idx ON issues_v2 (id) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS issues_v2_latest_pagination_idx ON issues_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS issues_v2_project_id_idx ON issues_v2 (project_id);
CREATE INDEX IF NOT EXISTS issues_v2_updated_at_id_idx ON issues_v2 (updated_at DESC, id DESC);
