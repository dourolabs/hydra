-- Add versioned storage for Store payload tables.
ALTER TABLE hydra.issues
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.issues
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.issues
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.issues
    DROP CONSTRAINT IF EXISTS issues_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'issues_id_version_unique'
        AND conrelid = 'hydra.issues'::regclass
    ) THEN
        ALTER TABLE hydra.issues
            ADD CONSTRAINT issues_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS issues_latest_idx
    ON hydra.issues (id, version_number DESC);

ALTER TABLE hydra.patches
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.patches
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.patches
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.patches
    DROP CONSTRAINT IF EXISTS patches_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'patches_id_version_unique'
        AND conrelid = 'hydra.patches'::regclass
    ) THEN
        ALTER TABLE hydra.patches
            ADD CONSTRAINT patches_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS patches_latest_idx
    ON hydra.patches (id, version_number DESC);

ALTER TABLE hydra.tasks
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.tasks
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.tasks
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.tasks
    DROP CONSTRAINT IF EXISTS tasks_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tasks_id_version_unique'
        AND conrelid = 'hydra.tasks'::regclass
    ) THEN
        ALTER TABLE hydra.tasks
            ADD CONSTRAINT tasks_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS tasks_latest_idx
    ON hydra.tasks (id, version_number DESC);

ALTER TABLE hydra.task_status_logs
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.task_status_logs
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.task_status_logs
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.task_status_logs
    DROP CONSTRAINT IF EXISTS task_status_logs_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'task_status_logs_id_version_unique'
        AND conrelid = 'hydra.task_status_logs'::regclass
    ) THEN
        ALTER TABLE hydra.task_status_logs
            ADD CONSTRAINT task_status_logs_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS task_status_logs_latest_idx
    ON hydra.task_status_logs (id, version_number DESC);

ALTER TABLE hydra.users
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.users
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.users
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.users
    DROP CONSTRAINT IF EXISTS users_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'users_id_version_unique'
        AND conrelid = 'hydra.users'::regclass
    ) THEN
        ALTER TABLE hydra.users
            ADD CONSTRAINT users_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS users_latest_idx
    ON hydra.users (id, version_number DESC);

ALTER TABLE hydra.repositories
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.repositories
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.repositories
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.repositories
    DROP CONSTRAINT IF EXISTS repositories_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'repositories_id_version_unique'
        AND conrelid = 'hydra.repositories'::regclass
    ) THEN
        ALTER TABLE hydra.repositories
            ADD CONSTRAINT repositories_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS repositories_latest_idx
    ON hydra.repositories (id, version_number DESC);

ALTER TABLE hydra.actors
    ADD COLUMN IF NOT EXISTS version_number BIGINT;
UPDATE hydra.actors
    SET version_number = 1
    WHERE version_number IS NULL;
ALTER TABLE hydra.actors
    ALTER COLUMN version_number SET DEFAULT 1,
    ALTER COLUMN version_number SET NOT NULL;
ALTER TABLE hydra.actors
    DROP CONSTRAINT IF EXISTS actors_pkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'actors_id_version_unique'
        AND conrelid = 'hydra.actors'::regclass
    ) THEN
        ALTER TABLE hydra.actors
            ADD CONSTRAINT actors_id_version_unique UNIQUE (id, version_number);
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS actors_latest_idx
    ON hydra.actors (id, version_number DESC);
