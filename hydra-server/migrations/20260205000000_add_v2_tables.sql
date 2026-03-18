-- Create v2 tables with proper column definitions for all domain types.
-- These tables exist alongside the existing JSONB-based tables to enable
-- a safe migration path from v1 to v2 storage.

--------------------------------------------------------------------------------
-- metis.issues_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.issues_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    issue_type TEXT NOT NULL,
    description TEXT NOT NULL,
    creator TEXT NOT NULL,
    progress TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    assignee TEXT,
    job_settings JSONB NOT NULL DEFAULT '{}',
    todo_list JSONB NOT NULL DEFAULT '[]',
    dependencies JSONB NOT NULL DEFAULT '[]',
    patches JSONB NOT NULL DEFAULT '[]',
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS issues_v2_status_idx
    ON metis.issues_v2 (status);

CREATE INDEX IF NOT EXISTS issues_v2_latest_idx
    ON metis.issues_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_issues_v2 ON metis.issues_v2;
CREATE TRIGGER set_timestamp_issues_v2
BEFORE UPDATE ON metis.issues_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.patches_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.patches_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL,
    diff TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    is_automatic_backup BOOLEAN NOT NULL DEFAULT FALSE,
    created_by TEXT,
    reviews JSONB NOT NULL DEFAULT '[]',
    service_repo_name TEXT NOT NULL,
    github JSONB,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS patches_v2_status_idx
    ON metis.patches_v2 (status);

CREATE INDEX IF NOT EXISTS patches_v2_latest_idx
    ON metis.patches_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_patches_v2 ON metis.patches_v2;
CREATE TRIGGER set_timestamp_patches_v2
BEFORE UPDATE ON metis.patches_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.tasks_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.tasks_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    prompt TEXT NOT NULL,
    context JSONB NOT NULL,
    spawned_from TEXT,
    image TEXT,
    model TEXT,
    env_vars JSONB NOT NULL DEFAULT '{}',
    cpu_limit TEXT,
    memory_limit TEXT,
    status TEXT NOT NULL DEFAULT 'complete',
    last_message TEXT,
    error JSONB,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS tasks_v2_spawned_from_idx
    ON metis.tasks_v2 (spawned_from);

CREATE INDEX IF NOT EXISTS tasks_v2_status_idx
    ON metis.tasks_v2 (status);

CREATE INDEX IF NOT EXISTS tasks_v2_latest_idx
    ON metis.tasks_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_tasks_v2 ON metis.tasks_v2;
CREATE TRIGGER set_timestamp_tasks_v2
BEFORE UPDATE ON metis.tasks_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.users_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.users_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    username TEXT NOT NULL,
    github_user_id BIGINT NOT NULL,
    github_token TEXT NOT NULL,
    github_refresh_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS users_v2_latest_idx
    ON metis.users_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_users_v2 ON metis.users_v2;
CREATE TRIGGER set_timestamp_users_v2
BEFORE UPDATE ON metis.users_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.actors_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.actors_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    auth_token_hash TEXT NOT NULL,
    auth_token_salt TEXT NOT NULL,
    user_or_worker JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS actors_v2_latest_idx
    ON metis.actors_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_actors_v2 ON metis.actors_v2;
CREATE TRIGGER set_timestamp_actors_v2
BEFORE UPDATE ON metis.actors_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.repositories_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.repositories_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    remote_url TEXT NOT NULL,
    default_branch TEXT,
    default_image TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS repositories_v2_latest_idx
    ON metis.repositories_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_repositories_v2 ON metis.repositories_v2;
CREATE TRIGGER set_timestamp_repositories_v2
BEFORE UPDATE ON metis.repositories_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

--------------------------------------------------------------------------------
-- metis.documents_v2
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.documents_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    body_markdown TEXT NOT NULL,
    path TEXT,
    created_by TEXT,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS documents_v2_path_idx
    ON metis.documents_v2 (path);

CREATE INDEX IF NOT EXISTS documents_v2_path_prefix_idx
    ON metis.documents_v2 USING btree (path text_pattern_ops);

CREATE INDEX IF NOT EXISTS documents_v2_latest_idx
    ON metis.documents_v2 (id, version_number DESC);

DROP TRIGGER IF EXISTS set_timestamp_documents_v2 ON metis.documents_v2;
CREATE TRIGGER set_timestamp_documents_v2
BEFORE UPDATE ON metis.documents_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();
