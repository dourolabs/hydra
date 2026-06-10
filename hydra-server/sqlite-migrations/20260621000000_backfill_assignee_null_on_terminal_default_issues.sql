-- Sister to the Postgres
-- `20260621000000_backfill_assignee_null_on_terminal_default_issues.sql`.
-- See that file for design notes.
--
-- SQLite differences vs. the postgres sibling:
--   * No `metis.` schema prefix.
--   * `is_latest` is INTEGER (1/0) rather than BOOLEAN.

UPDATE issues_v2
SET assignee = NULL,
    assignee_principal = NULL
WHERE is_latest = 1
  AND project_id = 'j-defaul'
  AND status_sequence IN (
      SELECT sequence
      FROM statuses
      WHERE project_id = 'j-defaul'
        AND key IN ('closed', 'dropped', 'failed')
  )
  AND (assignee IS NOT NULL OR assignee_principal IS NOT NULL);
