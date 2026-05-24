-- Tighten the new session-shape columns on tasks_v2 to NOT NULL.
--
-- These columns were added (nullable) and backfilled in
-- 20260523020000_add_session_shape_columns; PR-5 / Phase E step 16
-- (20260525000000_drop_legacy_session_columns) made them the sole source of
-- session shape and the in-Rust `TaskRow` now treats them as non-optional
-- (e.g. `mount_spec: String`, not `Option<_>`). This migration aligns the
-- schema with that in-Rust assumption.
--
-- `resumed_from` stays nullable: only resumed sessions reference a predecessor.
--
-- SQLite does not support `ALTER COLUMN ... SET NOT NULL`, so we follow the
-- standard recreate-table-and-copy dance. Columns are listed explicitly in
-- both INSERT and SELECT — never `INSERT INTO new SELECT * FROM old` (Hydra
-- has been bitten by positional-column drift before).

CREATE TABLE tasks_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    spawned_from TEXT,
    image TEXT,
    env_vars TEXT NOT NULL DEFAULT '{}',
    cpu_limit TEXT,
    memory_limit TEXT,
    status TEXT NOT NULL DEFAULT 'complete',
    last_message TEXT,
    error TEXT,
    secrets TEXT,
    creator TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    creation_time TEXT,
    start_time TEXT,
    end_time TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    conversation_id TEXT,
    usage TEXT DEFAULT NULL,
    mount_spec TEXT NOT NULL,
    agent_config TEXT NOT NULL,
    mode TEXT NOT NULL,
    resumed_from TEXT,
    PRIMARY KEY (id, version_number)
);

INSERT INTO tasks_v2_new (
    id,
    version_number,
    spawned_from,
    image,
    env_vars,
    cpu_limit,
    memory_limit,
    status,
    last_message,
    error,
    secrets,
    creator,
    deleted,
    actor,
    created_at,
    updated_at,
    creation_time,
    start_time,
    end_time,
    is_latest,
    conversation_id,
    usage,
    mount_spec,
    agent_config,
    mode,
    resumed_from
)
SELECT
    id,
    version_number,
    spawned_from,
    image,
    env_vars,
    cpu_limit,
    memory_limit,
    status,
    last_message,
    error,
    secrets,
    creator,
    deleted,
    actor,
    created_at,
    updated_at,
    creation_time,
    start_time,
    end_time,
    is_latest,
    conversation_id,
    usage,
    mount_spec,
    agent_config,
    mode,
    resumed_from
FROM tasks_v2;

DROP TABLE tasks_v2;
ALTER TABLE tasks_v2_new RENAME TO tasks_v2;

-- Recreate the indexes that lived on the original tasks_v2.
CREATE INDEX tasks_v2_spawned_from_idx        ON tasks_v2 (spawned_from);
CREATE INDEX tasks_v2_status_idx              ON tasks_v2 (status);
CREATE INDEX tasks_v2_latest_idx              ON tasks_v2 (id, version_number DESC);
CREATE INDEX tasks_v2_latest_id_idx           ON tasks_v2 (id) WHERE is_latest = 1;
CREATE INDEX tasks_v2_latest_pagination_idx   ON tasks_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
