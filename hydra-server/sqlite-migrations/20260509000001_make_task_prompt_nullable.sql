-- SQLite does not support ALTER COLUMN to drop NOT NULL, so we recreate the table.

-- 1. Create temporary table with prompt nullable
CREATE TABLE tasks_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    prompt TEXT,
    context TEXT NOT NULL,
    spawned_from TEXT,
    image TEXT,
    model TEXT,
    env_vars TEXT NOT NULL DEFAULT '{}',
    cpu_limit TEXT,
    memory_limit TEXT,
    status TEXT NOT NULL DEFAULT 'complete',
    last_message TEXT,
    error TEXT,
    secrets TEXT,
    mcp_config TEXT,
    creator TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    creation_time TEXT,
    start_time TEXT,
    end_time TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id, version_number)
);

-- 2. Copy data, converting empty prompts to NULL
INSERT INTO tasks_v2_new
SELECT id, version_number,
       CASE WHEN prompt = '' THEN NULL ELSE prompt END,
       context, spawned_from, image, model, env_vars,
       cpu_limit, memory_limit, status, last_message, error, secrets,
       mcp_config, creator, deleted, actor, created_at, updated_at,
       creation_time, start_time, end_time, is_latest
FROM tasks_v2;

-- 3. Drop old table and rename
DROP TABLE tasks_v2;
ALTER TABLE tasks_v2_new RENAME TO tasks_v2;

-- 4. Recreate indexes
CREATE INDEX IF NOT EXISTS tasks_v2_spawned_from_idx ON tasks_v2 (spawned_from);
CREATE INDEX IF NOT EXISTS tasks_v2_status_idx ON tasks_v2 (status);
CREATE INDEX IF NOT EXISTS tasks_v2_latest_idx ON tasks_v2 (id, version_number DESC);
CREATE INDEX IF NOT EXISTS tasks_v2_latest_id_idx ON tasks_v2 (id) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS tasks_v2_latest_pagination_idx ON tasks_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
