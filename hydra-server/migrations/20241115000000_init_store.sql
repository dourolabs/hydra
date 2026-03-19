-- Base schema for Postgres-backed Store objects.
CREATE SCHEMA IF NOT EXISTS hydra;

CREATE TABLE IF NOT EXISTS hydra.payload_schema_versions (
    object_type TEXT PRIMARY KEY,
    current_version INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (current_version > 0)
);

CREATE OR REPLACE FUNCTION hydra.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION hydra.current_schema_version(target TEXT)
RETURNS INTEGER AS $$
DECLARE
    version INTEGER;
BEGIN
    SELECT current_version INTO version
    FROM hydra.payload_schema_versions
    WHERE object_type = target;

    RETURN COALESCE(version, 1);
END;
$$ LANGUAGE plpgsql STABLE;

DROP TRIGGER IF EXISTS set_timestamp_payload_schema_versions ON hydra.payload_schema_versions;
CREATE TRIGGER set_timestamp_payload_schema_versions
BEFORE UPDATE ON hydra.payload_schema_versions
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

INSERT INTO hydra.payload_schema_versions (object_type, current_version)
VALUES
    ('issue', 1),
    ('patch', 1),
    ('task', 1),
    ('task_status_log', 1),
    ('user', 1),
    ('repository', 1)
ON CONFLICT (object_type) DO NOTHING;

-- Placeholder hook for evolving JSON payloads without breaking reads.
CREATE OR REPLACE FUNCTION hydra.migrate_payload(
    object_type TEXT,
    from_version INTEGER,
    to_version INTEGER,
    payload JSONB
) RETURNS JSONB AS $$
BEGIN
    -- Future migrations can branch on object_type/from_version/to_version
    -- and transform the payload accordingly.
    RETURN payload;
END;
$$ LANGUAGE plpgsql STABLE;

CREATE TABLE IF NOT EXISTS hydra.issues (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('issue'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_issues ON hydra.issues;
CREATE TRIGGER set_timestamp_issues
BEFORE UPDATE ON hydra.issues
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

CREATE TABLE IF NOT EXISTS hydra.patches (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('patch'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_patches ON hydra.patches;
CREATE TRIGGER set_timestamp_patches
BEFORE UPDATE ON hydra.patches
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

CREATE TABLE IF NOT EXISTS hydra.tasks (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('task'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_tasks ON hydra.tasks;
CREATE TRIGGER set_timestamp_tasks
BEFORE UPDATE ON hydra.tasks
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

CREATE TABLE IF NOT EXISTS hydra.task_status_logs (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('task_status_log'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_task_status_logs ON hydra.task_status_logs;
CREATE TRIGGER set_timestamp_task_status_logs
BEFORE UPDATE ON hydra.task_status_logs
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

CREATE TABLE IF NOT EXISTS hydra.users (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('user'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_users ON hydra.users;
CREATE TRIGGER set_timestamp_users
BEFORE UPDATE ON hydra.users
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();

CREATE TABLE IF NOT EXISTS hydra.repositories (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('repository'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_repositories ON hydra.repositories;
CREATE TRIGGER set_timestamp_repositories
BEFORE UPDATE ON hydra.repositories
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();
