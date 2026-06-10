-- Drop the `is_assignment_agent` column from the agents table.
-- The assignment-agent concept has been removed; per-status
-- `on_enter.assign_to` (`hydra-common::api::v1::projects::StatusOnEnter`)
-- is now the canonical mechanism for routing issues to an agent.
--
-- SQLite has no `ALTER TABLE ... DROP COLUMN` that works for our minimum
-- supported version, so we use the rebuild recipe: create a replacement
-- table with the new shape, copy rows over with explicit column lists,
-- drop the old table, and rename. The agents table has no foreign keys
-- pointing in, so this is safe.

CREATE TABLE agents_new (
    name TEXT PRIMARY KEY,
    prompt_path TEXT NOT NULL,
    max_tries INTEGER NOT NULL DEFAULT 3,
    max_simultaneous INTEGER NOT NULL DEFAULT 2147483647,
    deleted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    secrets TEXT NOT NULL DEFAULT '[]',
    mcp_config_path TEXT DEFAULT NULL,
    is_default_conversation_agent INTEGER NOT NULL DEFAULT 0
);

INSERT INTO agents_new (
    name,
    prompt_path,
    max_tries,
    max_simultaneous,
    deleted,
    created_at,
    updated_at,
    secrets,
    mcp_config_path,
    is_default_conversation_agent
)
SELECT
    name,
    prompt_path,
    max_tries,
    max_simultaneous,
    deleted,
    created_at,
    updated_at,
    secrets,
    mcp_config_path,
    is_default_conversation_agent
FROM agents;

DROP TABLE agents;

ALTER TABLE agents_new RENAME TO agents;
