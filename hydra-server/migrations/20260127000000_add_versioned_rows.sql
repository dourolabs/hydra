-- Add versioned storage for Store payload tables.
ALTER TABLE metis.issues
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.issues
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.issues
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.issues
    DROP CONSTRAINT IF EXISTS issues_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'issues_id_version_unique'
        AND conrelid = 'metis.issues'::regclass
    ) THEN
        ALTER TABLE metis.issues
            ADD CONSTRAINT issues_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS issues_latest_idx
    ON metis.issues (id, version_number DESC);

ALTER TABLE metis.patches
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.patches
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.patches
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.patches
    DROP CONSTRAINT IF EXISTS patches_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'patches_id_version_unique'
        AND conrelid = 'metis.patches'::regclass
    ) THEN
        ALTER TABLE metis.patches
            ADD CONSTRAINT patches_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS patches_latest_idx
    ON metis.patches (id, version_number DESC);

ALTER TABLE metis.tasks
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.tasks
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.tasks
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.tasks
    DROP CONSTRAINT IF EXISTS tasks_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tasks_id_version_unique'
        AND conrelid = 'metis.tasks'::regclass
    ) THEN
        ALTER TABLE metis.tasks
            ADD CONSTRAINT tasks_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS tasks_latest_idx
    ON metis.tasks (id, version_number DESC);

ALTER TABLE metis.task_status_logs
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.task_status_logs
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.task_status_logs
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.task_status_logs
    DROP CONSTRAINT IF EXISTS task_status_logs_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'task_status_logs_id_version_unique'
        AND conrelid = 'metis.task_status_logs'::regclass
    ) THEN
        ALTER TABLE metis.task_status_logs
            ADD CONSTRAINT task_status_logs_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS task_status_logs_latest_idx
    ON metis.task_status_logs (id, version_number DESC);

ALTER TABLE metis.users
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.users
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.users
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.users
    DROP CONSTRAINT IF EXISTS users_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'users_id_version_unique'
        AND conrelid = 'metis.users'::regclass
    ) THEN
        ALTER TABLE metis.users
            ADD CONSTRAINT users_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS users_latest_idx
    ON metis.users (id, version_number DESC);

ALTER TABLE metis.repositories
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.repositories
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.repositories
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.repositories
    DROP CONSTRAINT IF EXISTS repositories_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'repositories_id_version_unique'
        AND conrelid = 'metis.repositories'::regclass
    ) THEN
        ALTER TABLE metis.repositories
            ADD CONSTRAINT repositories_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS repositories_latest_idx
    ON metis.repositories (id, version_number DESC);

ALTER TABLE metis.actors
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE metis.actors
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE metis.actors
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE metis.actors
    DROP CONSTRAINT IF EXISTS actors_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'actors_id_version_unique'
        AND conrelid = 'metis.actors'::regclass
    ) THEN
        ALTER TABLE metis.actors
            ADD CONSTRAINT actors_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS actors_latest_idx
    ON metis.actors (id, version_number DESC);
