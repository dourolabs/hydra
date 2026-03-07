-- SQLite schema: all 13 tables for the metis store.
-- Uses TEXT for timestamps (ISO8601), INTEGER for booleans, TEXT for JSON, BLOB for binary.

--------------------------------------------------------------------------------
-- repositories_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS repositories_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    remote_url TEXT NOT NULL,
    default_branch TEXT,
    default_image TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    patch_workflow TEXT,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS repositories_v2_latest_idx
    ON repositories_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- actors_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS actors_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    auth_token_hash TEXT NOT NULL,
    auth_token_salt TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    creator TEXT,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS actors_v2_latest_idx
    ON actors_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- users_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    username TEXT NOT NULL,
    github_user_id INTEGER NOT NULL,
    github_token TEXT,
    github_refresh_token TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS users_v2_latest_idx
    ON users_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- issues_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS issues_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    issue_type TEXT NOT NULL,
    description TEXT NOT NULL,
    creator TEXT NOT NULL,
    progress TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    assignee TEXT,
    job_settings TEXT NOT NULL DEFAULT '{}',
    todo_list TEXT NOT NULL DEFAULT '[]',
    dependencies TEXT NOT NULL DEFAULT '[]',
    patches TEXT NOT NULL DEFAULT '[]',
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS issues_v2_status_idx ON issues_v2 (status);
CREATE INDEX IF NOT EXISTS issues_v2_latest_idx ON issues_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- patches_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS patches_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL,
    diff TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    is_automatic_backup INTEGER NOT NULL DEFAULT 0,
    created_by TEXT,
    creator TEXT,
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
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS patches_v2_status_idx ON patches_v2 (status);
CREATE INDEX IF NOT EXISTS patches_v2_latest_idx ON patches_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- tasks_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tasks_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    prompt TEXT NOT NULL,
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
    creator TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS tasks_v2_spawned_from_idx ON tasks_v2 (spawned_from);
CREATE INDEX IF NOT EXISTS tasks_v2_status_idx ON tasks_v2 (status);
CREATE INDEX IF NOT EXISTS tasks_v2_latest_idx ON tasks_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- documents_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS documents_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    body_markdown TEXT NOT NULL,
    path TEXT,
    created_by TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS documents_v2_path_idx ON documents_v2 (path);
CREATE INDEX IF NOT EXISTS documents_v2_latest_idx ON documents_v2 (id, version_number DESC);

--------------------------------------------------------------------------------
-- messages_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS messages_v2 (
    id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    sender TEXT,
    recipient TEXT NOT NULL,
    body TEXT NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_messages_v2_latest ON messages_v2 (id, version_number DESC);
CREATE INDEX IF NOT EXISTS idx_messages_v2_sender ON messages_v2 (sender);
CREATE INDEX IF NOT EXISTS idx_messages_v2_recipient ON messages_v2 (recipient);

--------------------------------------------------------------------------------
-- notifications
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS notifications (
    id TEXT NOT NULL PRIMARY KEY,
    recipient TEXT NOT NULL,
    source_actor TEXT,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    source_issue_id TEXT,
    policy TEXT NOT NULL DEFAULT 'walk_up',
    is_read INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_notifications_recipient_unread
    ON notifications (recipient, is_read, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_recipient_all
    ON notifications (recipient, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_object
    ON notifications (object_id, object_version);

--------------------------------------------------------------------------------
-- agents
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS agents (
    name TEXT PRIMARY KEY,
    prompt_path TEXT NOT NULL,
    max_tries INTEGER NOT NULL DEFAULT 3,
    max_simultaneous INTEGER NOT NULL DEFAULT 2147483647,
    is_assignment_agent INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now'))
);

-- SQLite doesn't support partial unique indexes with WHERE, so enforce at app level.

--------------------------------------------------------------------------------
-- labels
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS labels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#6b7280',
    recurse INTEGER NOT NULL DEFAULT 1,
    hidden INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now'))
);

--------------------------------------------------------------------------------
-- label_associations
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS label_associations (
    label_id TEXT NOT NULL REFERENCES labels(id),
    object_id TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (label_id, object_id)
);

CREATE INDEX IF NOT EXISTS label_associations_object_idx
    ON label_associations (object_id);

--------------------------------------------------------------------------------
-- user_secrets
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS user_secrets (
    username TEXT NOT NULL,
    secret_name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (username, secret_name)
);
