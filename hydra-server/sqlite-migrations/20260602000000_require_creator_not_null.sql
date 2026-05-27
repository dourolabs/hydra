-- Backfill any remaining NULL creator rows and enforce NOT NULL on the
-- `creator` column for the three affected v2 tables. SQLite has no
-- `ALTER COLUMN ... SET NOT NULL`, so we use the standard
-- create-new-table-and-copy dance. Columns are listed explicitly in both
-- INSERT and SELECT — never `INSERT INTO new SELECT * FROM old` (positional
-- column drift has bitten Hydra before).

--------------------------------------------------------------------------------
-- Defensive backfill (idempotent — earlier backfills already ran).
--------------------------------------------------------------------------------
UPDATE actors_v2  SET creator = 'unknown' WHERE creator IS NULL;
UPDATE patches_v2 SET creator = 'unknown' WHERE creator IS NULL;
UPDATE tasks_v2   SET creator = 'unknown' WHERE creator IS NULL;

--------------------------------------------------------------------------------
-- actors_v2
--------------------------------------------------------------------------------
CREATE TABLE actors_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    auth_token_hash TEXT NOT NULL,
    auth_token_salt TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    creator TEXT NOT NULL,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    is_latest INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id, version_number)
);

INSERT INTO actors_v2_new (
    id,
    version_number,
    auth_token_hash,
    auth_token_salt,
    actor_id,
    creator,
    actor,
    created_at,
    updated_at,
    is_latest
)
SELECT
    id,
    version_number,
    auth_token_hash,
    auth_token_salt,
    actor_id,
    creator,
    actor,
    created_at,
    updated_at,
    is_latest
FROM actors_v2;

DROP TABLE actors_v2;
ALTER TABLE actors_v2_new RENAME TO actors_v2;

CREATE INDEX actors_v2_latest_idx            ON actors_v2 (id, version_number DESC);
CREATE INDEX actors_v2_latest_id_idx         ON actors_v2 (id) WHERE is_latest = 1;
CREATE INDEX actors_v2_latest_pagination_idx ON actors_v2 (created_at DESC, id DESC) WHERE is_latest = 1;

--------------------------------------------------------------------------------
-- patches_v2
--------------------------------------------------------------------------------
CREATE TABLE patches_v2_new (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL,
    diff TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    is_automatic_backup INTEGER NOT NULL DEFAULT 0,
    creator TEXT NOT NULL,
    base_branch TEXT,
    branch_name TEXT,
    commit_range TEXT,
    reviews TEXT NOT NULL DEFAULT '[]',
    service_repo_name TEXT NOT NULL,
    github TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    is_latest INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id, version_number)
);

INSERT INTO patches_v2_new (
    id,
    version_number,
    title,
    description,
    diff,
    status,
    is_automatic_backup,
    creator,
    base_branch,
    branch_name,
    commit_range,
    reviews,
    service_repo_name,
    github,
    deleted,
    actor,
    created_at,
    updated_at,
    is_latest
)
SELECT
    id,
    version_number,
    title,
    description,
    diff,
    status,
    is_automatic_backup,
    creator,
    base_branch,
    branch_name,
    commit_range,
    reviews,
    service_repo_name,
    github,
    deleted,
    actor,
    created_at,
    updated_at,
    is_latest
FROM patches_v2;

DROP TABLE patches_v2;
ALTER TABLE patches_v2_new RENAME TO patches_v2;

CREATE INDEX patches_v2_status_idx            ON patches_v2 (status);
CREATE INDEX patches_v2_latest_idx            ON patches_v2 (id, version_number DESC);
CREATE INDEX patches_v2_latest_id_idx         ON patches_v2 (id) WHERE is_latest = 1;
CREATE INDEX patches_v2_latest_pagination_idx ON patches_v2 (created_at DESC, id DESC) WHERE is_latest = 1;

--------------------------------------------------------------------------------
-- tasks_v2 (largest schema; mirrors the shape established by
-- 20260526000000_require_session_shape_columns_not_null.sql plus the
-- creator-NOT-NULL change).
--------------------------------------------------------------------------------
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
    creator TEXT NOT NULL,
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

CREATE INDEX tasks_v2_spawned_from_idx        ON tasks_v2 (spawned_from);
CREATE INDEX tasks_v2_status_idx              ON tasks_v2 (status);
CREATE INDEX tasks_v2_latest_idx              ON tasks_v2 (id, version_number DESC);
CREATE INDEX tasks_v2_latest_id_idx           ON tasks_v2 (id) WHERE is_latest = 1;
CREATE INDEX tasks_v2_latest_pagination_idx   ON tasks_v2 (created_at DESC, id DESC) WHERE is_latest = 1;
