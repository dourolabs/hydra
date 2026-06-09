-- baseline-version: 20260611000000
-- Postgres pre-issues-v2-project-id-not-null baseline. INSERTs are
-- valid against the schema state at postgres migration
-- `20260611000000_drop_projects_default_status_key.sql`, immediately
-- before `20260612000000_issues_v2_project_id_not_null.sql` tightens
-- `metis.issues_v2.project_id` to NOT NULL via `ALTER COLUMN`.
--
-- Scope: per [[i-glruodtb]], the NOT NULL migration must survive on a
-- non-default fixture row. Seed a single non-default `issues_v2` row
-- whose `project_id` is set to the seeded `j-defaul` project: the row
-- must survive the column tightening untouched, the resulting column
-- must be NOT NULL, fresh NULL inserts must be rejected, and the
-- migration body must re-execute cleanly (idempotency).
--
-- The fresh-pool null-baseline guard (the migration's pre-flight
-- `DO $$ ... RAISE EXCEPTION` block firing on a stale NULL row) is
-- only exercised by the sister sqlite roundtrip; the postgres roundtrip
-- shares a single database across tests, so resetting it mid-run to
-- seed a NULL row would invalidate the downstream idempotency
-- assertions at the tail of the test.

-- `is_latest` is set by the `versioned_set_is_latest` BEFORE-INSERT
-- trigger on `metis.issues_v2`; omit the column from the INSERT instead
-- of setting it explicitly, matching the sibling postgres baselines.
INSERT INTO metis.issues_v2 (
    id,
    version_number,
    issue_type,
    description,
    creator,
    project_id
) VALUES (
    'i-prjidnn',
    1,
    'task',
    'project_id NOT NULL baseline fixture row',
    'jayantk',
    'j-defaul'
);
