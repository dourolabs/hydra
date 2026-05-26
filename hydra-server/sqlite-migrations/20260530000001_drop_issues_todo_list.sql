-- Drop the `todo_list` JSON column from `issues_v2`. The issue todo-list
-- feature has been fully removed (HTTP API, frontend, types, automations,
-- and stores). Any column data is discarded.
ALTER TABLE issues_v2 DROP COLUMN todo_list;
