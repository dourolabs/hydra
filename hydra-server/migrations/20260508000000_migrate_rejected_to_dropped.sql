-- Migrate all issues with 'rejected' status to 'dropped'.
-- The 'rejected' status variant has been removed from the codebase.
UPDATE metis.issues_v2 SET status = 'dropped' WHERE status = 'rejected';
