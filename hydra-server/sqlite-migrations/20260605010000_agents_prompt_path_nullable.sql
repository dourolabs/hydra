-- Drop the NOT NULL constraint on agents.prompt_path so the column can
-- carry NULL for "no path", instead of the legacy empty-string sentinel.
-- Backfill any existing empty-string rows to NULL.
--
-- SQLite does not support `ALTER COLUMN ... DROP NOT NULL`, so we follow
-- the standard recreate-table-and-copy dance. Columns are listed
-- explicitly in both INSERT and SELECT — never `INSERT INTO new SELECT *
-- FROM old` (Hydra has been bitten by positional-column drift before).

CREATE TABLE agents_new (
    name TEXT PRIMARY KEY,
    prompt_path TEXT,
    max_tries INTEGER NOT NULL DEFAULT 3,
    max_simultaneous INTEGER NOT NULL DEFAULT 2147483647,
    is_assignment_agent INTEGER NOT NULL DEFAULT 0,
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
    is_assignment_agent,
    deleted,
    created_at,
    updated_at,
    secrets,
    mcp_config_path,
    is_default_conversation_agent
)
SELECT
    name,
    NULLIF(prompt_path, ''),
    max_tries,
    max_simultaneous,
    is_assignment_agent,
    deleted,
    created_at,
    updated_at,
    secrets,
    mcp_config_path,
    is_default_conversation_agent
FROM agents;

DROP TABLE agents;
ALTER TABLE agents_new RENAME TO agents;
