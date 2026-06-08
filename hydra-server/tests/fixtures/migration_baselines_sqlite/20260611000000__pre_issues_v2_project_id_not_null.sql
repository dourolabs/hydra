-- baseline-version: 20260611000000
-- SQLite pre-issues-v2-project-id-not-null baseline. INSERTs are valid
-- against the schema state at sqlite migration
-- `20260611000000_drop_projects_default_status_key.sql`, immediately
-- before `20260612000000_issues_v2_project_id_not_null.sql` tightens
-- `issues_v2.project_id` to NOT NULL via the table-rebuild dance.
--
-- Scope: per [[i-glruodtb]], the NOT NULL migration must survive the
-- table rebuild on a non-default fixture row. Seed a single non-default
-- `issues_v2` row whose `project_id` is set to the seeded `j-defaul`
-- project: the row must round-trip through the rebuild unchanged, the
-- resulting table must reject fresh NULL inserts, and the migration
-- body must re-execute cleanly (idempotency).
--
-- The fresh-pool null-baseline guard (rejecting a stale NULL row at
-- pre-flight) is exercised separately in
-- `migration_roundtrip_sqlite::issues_v2_project_id_not_null_migration_rejects_null_baseline`
-- against a clean in-memory pool — running it against the shared
-- baseline pool here would block the downstream idempotency rerun.

INSERT INTO issues_v2 (
    id,
    version_number,
    issue_type,
    description,
    creator,
    is_latest,
    project_id
) VALUES (
    'i-prjidnn',
    1,
    'task',
    'project_id NOT NULL baseline fixture row',
    'jayantk',
    1,
    'j-defaul'
);
