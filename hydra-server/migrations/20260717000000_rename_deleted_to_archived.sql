-- Postgres sister to
-- `sqlite-migrations/20260717000000_rename_deleted_to_archived.sql`. See
-- that file for the rationale and scope.
--
-- Postgres `ALTER TABLE ... RENAME COLUMN` is in-place and preserves the
-- partial unique indexes that reference the column (the index continues to
-- reference the renamed column by oid). No data movement, so the
-- [[migrations]] guardrail against `INSERT INTO new_table SELECT *` does not
-- apply.

ALTER TABLE metis.repositories_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.users_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.issues_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.patches_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.tasks_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.documents_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.agents RENAME COLUMN deleted TO archived;
ALTER TABLE metis.labels RENAME COLUMN deleted TO archived;
ALTER TABLE metis.conversations_v2 RENAME COLUMN deleted TO archived;
ALTER TABLE metis.triggers RENAME COLUMN deleted TO archived;
