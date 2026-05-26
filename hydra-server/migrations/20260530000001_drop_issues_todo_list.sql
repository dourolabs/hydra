-- Drop the `todo_list` JSONB column from `metis.issues_v2`. The issue
-- todo-list feature has been fully removed (HTTP API, frontend, types,
-- automations, and stores). Any column data is discarded.
ALTER TABLE metis.issues_v2
    DROP COLUMN IF EXISTS todo_list;
