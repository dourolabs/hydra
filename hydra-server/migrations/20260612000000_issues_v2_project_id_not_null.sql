-- Tighten `metis.issues_v2.project_id` to NOT NULL.
--
-- The sibling `20260607000000_seed_default_project.sql` migration seeded
-- the `j-defaul` project row and backfilled every legacy NULL
-- `issues_v2.project_id` to it. This migration enforces the invariant at
-- the schema level so the Rust type cutover (Issue.project_id:
-- Option<ProjectId> -> ProjectId, IssueRow.project_id: Option<String> ->
-- String) cannot regress.
--
-- Pre-flight guard: refuse to run if any NULL `project_id` rows remain.
-- The backfill in 20260607000000 should have cleared them; this is
-- belt-and-suspenders so a stale environment fails loud instead of
-- silently coercing rows to the default project.

DO $$
DECLARE
    null_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO null_count FROM metis.issues_v2 WHERE project_id IS NULL;
    IF null_count > 0 THEN
        RAISE EXCEPTION
            'issues_v2_project_id_not_null: refusing to tighten column; % NULL project_id row(s) still present. Re-run the 20260607000000_seed_default_project backfill.',
            null_count;
    END IF;
END $$;

ALTER TABLE metis.issues_v2 ALTER COLUMN project_id SET NOT NULL;
